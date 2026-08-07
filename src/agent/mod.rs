//! Cooperative agent shell (post-M5 productization).
//!
//! An **agent** is an EL1 scheduled body that owns an [`AddressSpace`] and may
//! enter EL0 through [`crate::arch::el0`] sessions. Matches ADR-0006
//! (cooperative) and ADR-0014 (kernel `TTBR0` on lower-EL return).
//!
//! Sessions support **SVC resume** and **IRQ resume** (architectural):
//! - `svc #0` ([`kernel_core::syscall::SYS_PING`]) — count and resume
//! - `svc #1` ([`kernel_core::syscall::SYS_EXIT`]) — end session
//! - `svc #3` ([`kernel_core::syscall::SYS_SEND`]) — message through a slot (console = M8)
//! - [`el0::El0Outcome::Irq`] — [`crate::irq::handle_cpu_irq`], then resume at
//!   the interrupted insn (no software ELR skip)
//!
//! Default entry masks IRQs in EL0. Call [`el0::set_entry_irqs_unmasked`] before
//! a session that should take lower-EL IRQs.
//!
//! ## One session per agent, not per machine
//!
//! The session state belongs to the task (ADR-0017 §1), so the loop below runs
//! against `sched::current_el0_session()` and every `arch::el0` call checks that
//! it is the published one. Until ADR-0017 this was a single machine-wide slot,
//! and "do not `yield_now` inside a session" was a rule with nothing enforcing
//! it — a yield would have let the next agent overwrite this one's saved
//! context. That rule is gone: a second agent has a second session.
//!
//! ## The mask is one step, not the session
//!
//! The loop used to run inside a single `cpu::without_irqs` spanning the whole
//! session. It cannot, once `SYS_RECV` parks (ADR-0022 §2): `without_irqs` saves
//! `DAIF` on entry and restores it on exit, so a region containing a task switch
//! hands the next task this task's mask and later restores a value captured in
//! an epoch that has ended.
//!
//! So the mask wraps `enter_step` / `resume_step` / `end_step` — everything
//! `arch::el0` requires it for — and nothing else. The body between two steps
//! runs unmasked, which is where the park happens. Saved-register access needs
//! no mask: it is plain memory in the running task's own TCB, and every
//! `arch::el0` call checks the session is the published one.
//!
//! `scripts/check/irq-scope.sh` is what keeps the rule from being remembered
//! rather than true.

use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::syscall::{self, Status, Syscall};

use crate::arch::{cpu, el0};

use crate::ipc;
use crate::irq;
use crate::mm::{self, AddressSpace, AsError};
use crate::sched;

/// Why agent create / EL0 entry failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentError {
    As(AsError),
    Poke,
}

impl From<AsError> for AgentError {
    fn from(e: AsError) -> Self {
        Self::As(e)
    }
}

/// How an EL0 session finished.
///
/// A faulting agent used to be indistinguishable from a clean exit: both
/// returned `Ok(stats)` and the `ESR`/`FAR` that `El0Outcome::DataAbort`
/// carries were dropped on the floor. This does not decide what to *do* about a
/// fault — who kills the agent, who restarts it, what is counted — which is the
/// agent fault policy ADR-0016 names as missing. It stops the kernel from
/// throwing away the only evidence that a fault happened.
/// Why an EL0 session stopped (ADR-0018).
///
/// `#[must_use]` because ignoring one is the bug this type exists to prevent.
/// The kernel ends the *session* — that is mechanism, and it is not optional —
/// but what happens to the *task* is the creator's decision, and a creator that
/// drops this value has not made it. Before `SessionEnd` existed a faulting
/// agent returned `Ok(stats)` and its `ESR`/`FAR` went nowhere; the attribute is
/// what keeps that from being reachable again by inattention.
#[must_use = "the creator decides what happens to a faulting agent; the kernel only ended its session"]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionEnd {
    /// `SYS_EXIT`: the agent asked to stop.
    #[default]
    Exit,
    /// An SVC this kernel does not implement. The agent's error, not a fault.
    UnknownSvc { imm: u16 },
    /// The agent took a fault at EL0. Session is over and cannot be resumed.
    Fault { esr: u64, far: u64 },
}

/// EL0 faults since boot, machine-wide (ADR-0018 §3).
///
/// The kernel counts and does not act. The number exists so a fault is visible
/// to something other than the immediate caller — the boot oracle, chiefly,
/// which is how anything gets verified here. Same shape and same reason as
/// `sched::pending_overwrites`.
static FAULTS: AtomicU32 = AtomicU32::new(0);

fn note_fault() {
    FAULTS.fetch_add(1, Ordering::Relaxed);
}

/// EL0 faults since boot.
#[inline]
pub fn fault_count() -> u32 {
    FAULTS.load(Ordering::Relaxed)
}

/// Counters from one multi-event EL0 session.
///
/// `#[must_use]` for the reason [`SessionEnd`] is: the outcome it carries is the
/// creator's to act on, and these have been returned into `let _ =` before.
#[must_use = "the session outcome inside is the creator's decision to make"]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionStats {
    pub pings: u32,
    pub irqs: u32,
    /// Messages the agent sent through a slot it holds.
    pub sends: u32,
    /// Messages the agent took through a slot it holds.
    pub recvs: u32,
    /// `SYS_TRY_RECV` calls answered `Empty`.
    ///
    /// Counted so a creator can assert the non-blocking path *was* taken. Since
    /// `SYS_RECV` waits (ADR-0022 §1), `Empty` has exactly one producer left,
    /// and a status no program can reach is a status that stops being
    /// maintained.
    pub recv_empties: u32,
    /// Calls refused because the agent named authority it does not have.
    ///
    /// The number the boot oracle asserts. A protection nobody has seen fire is
    /// an assumption, so the good path is expected to contain one of these.
    pub authority_refusals: u32,
    /// Why the session stopped. See [`SessionEnd`].
    pub end: SessionEnd,
}

/// Enter EL0 with EL1 IRQs masked, and nothing else inside the mask.
///
/// `before_enter` shares this step deliberately: a setup that arms a soon
/// deadline must do it with IRQs already masked, or the tick is claimed by
/// `exception_irq_el1` and never observed as [`el0::El0Outcome::Irq`].
///
/// # Safety
///
/// `root`/`entry`/`sp` must describe a prepared address space, and `session`
/// must be the running task's — which [`el0::enter`] checks rather than assumes.
fn enter_step(
    session: *mut el0::El0Session,
    root: usize,
    entry: u64,
    sp: u64,
    before_enter: impl FnOnce(),
) -> el0::El0Outcome {
    cpu::without_irqs(|| {
        before_enter();
        // SAFETY: the caller's contract, plus the mask this closure holds.
        unsafe { el0::enter(session, root, entry, sp) }
    })
}

/// Resume a live session with EL1 IRQs masked, and nothing else inside the mask.
///
/// The masked region is **one step**, not the session (ADR-0022 §2).
/// `cpu::without_irqs` saves `DAIF` on entry and restores it on exit, so a
/// region that spanned a task switch would hand the next task this task's mask
/// and later restore a value captured in an epoch that has ended. The session
/// loop parks on `SYS_RECV`; that park is a switch; so the mask cannot span it.
fn resume_step(session: *mut el0::El0Session) -> el0::El0Outcome {
    // SAFETY: the caller only reaches here after a resumable outcome, and the
    // mask this closure holds is what `el0::resume` requires.
    cpu::without_irqs(|| unsafe { el0::resume(session) })
}

/// End a session with EL1 IRQs masked. Same one-step rule as [`resume_step`].
fn end_step(session: *mut el0::El0Session) {
    // SAFETY: called on the paths that will not resume — the session is either
    // finished by `SYS_EXIT`/an unimplemented SVC, or already ended by the
    // vector path after a fault. Ending under the mask is what clears
    // `el0_kernel_ttbr0` while nothing can fault.
    cpu::without_irqs(|| unsafe { el0::end_session(session) });
}

/// Turn a recv result into the agent's `x0`, writing the payload on success.
///
/// Shared by the two recv calls because only the *waiting* differs. On anything
/// but `Ok` the kernel writes no payload, so `x1..x3` keep whatever the agent
/// itself left there: the reply registers are meaningful only when `x0` says
/// `Ok`, and the kernel does not clear an agent's own registers to make a point.
fn recv_reply(
    session: *mut el0::El0Session,
    result: Result<ipc::Message, ipc::RecvError>,
    stats: &mut SessionStats,
) -> Status {
    match result {
        Ok(msg) => {
            stats.recvs = stats.recvs.saturating_add(1);
            el0::set_saved_gpr(session, 1, u64::from(msg.tag));
            el0::set_saved_gpr(session, 2, msg.a);
            el0::set_saved_gpr(session, 3, msg.b);
            Status::Ok
        }
        Err(ipc::RecvError::Empty) => {
            stats.recv_empties = stats.recv_empties.saturating_add(1);
            Status::Empty
        }
        // Someone else is already waiting on this endpoint. Not an authority
        // violation — the agent holds what it named — so it does not touch the
        // counter the boot check asserts exactly (ADR-0022 §4).
        Err(ipc::RecvError::Busy) => Status::Busy,
        Err(ipc::RecvError::Cancelled) => Status::Cancelled,
        Err(ipc::RecvError::BadCap) => {
            stats.authority_refusals = stats.authority_refusals.saturating_add(1);
            Status::Authority
        }
    }
}

/// EL1-owned user address space ready for one-shot EL0 entry.
pub struct Agent {
    aspace: AddressSpace,
}

impl Agent {
    /// Allocate, prepare (kernel clone + user window).
    pub fn create_prepared() -> Result<Self, AgentError> {
        let mut aspace = AddressSpace::create()?;
        aspace.prepare_for_el0()?;
        Ok(Self { aspace })
    }

    /// Take an address space the caller already built.
    ///
    /// The loader's entry point: a manifest entry's geometry and device grant
    /// are applied to the address space before an agent exists to own it
    /// (ADR-0021 §5).
    pub fn from_aspace(aspace: AddressSpace) -> Self {
        Self { aspace }
    }

    #[inline]
    pub fn aspace_mut(&mut self) -> &mut AddressSpace {
        &mut self.aspace
    }

    /// Write user text and enter EL0 until the first lower-EL sync (one-shot).
    pub fn run_user_prog(&mut self, prog: &[u8]) -> Result<el0::El0Outcome, AgentError> {
        self.aspace
            .poke_user(0, prog)
            .map_err(|_| AgentError::Poke)?;
        // SAFETY: the AS was prepared by `create_prepared`, the whole entry runs
        // inside `without_irqs` so EL1 interrupts are masked as `el0::run`
        // requires, and one session at a time holds because nothing in this
        // closure yields (ADR-0016).
        let outcome = cpu::without_irqs(|| unsafe {
            el0::run(
                sched::current_el0_session(),
                self.aspace.root_phys(),
                self.aspace.user_entry_va(),
                self.aspace.user_sp(),
            )
        });
        Ok(outcome)
    }

    /// Multi-event session: resume after Ping / Send / Recv / Irq; stop on Exit / fault.
    ///
    /// IRQ resume re-executes the interrupted instruction (architectural). User
    /// text must make forward progress itself (e.g. a finite spin whose GPRs
    /// survive the save/restore).
    pub fn run_user_prog_resuming(&mut self, prog: &[u8]) -> Result<SessionStats, AgentError> {
        self.run_user_prog_resuming_prep(prog, || {})
    }

    /// Like [`run_user_prog_resuming`], but runs `before_enter` with EL1 IRQs
    /// already masked, immediately before [`el0::enter`].
    ///
    /// Required for setups that arm a soon timer deadline: if that ran with
    /// IRQs open at EL1, the tick would be claimed by `exception_irq_el1` and
    /// never observed as [`el0::El0Outcome::Irq`]. That is why `before_enter`
    /// shares the entry's masked step rather than running before it.
    pub fn run_user_prog_resuming_prep(
        &mut self,
        prog: &[u8],
        before_enter: impl FnOnce(),
    ) -> Result<SessionStats, AgentError> {
        self.aspace
            .poke_user(0, prog)
            .map_err(|_| AgentError::Poke)?;
        let root = self.aspace.root_phys();
        let entry = self.aspace.user_entry_va();
        let sp = self.aspace.user_sp();
        // The session this task owns. Every call below passes it, and
        // `arch::el0` refuses to act on a session the assembly would not
        // see — a switch that stopped publishing panics there instead of
        // handing this loop another task's saved registers (ADR-0017 §1).
        // It survives a park because the session lives in the TCB and the
        // scheduler republishes on every switch-in.
        let session = sched::current_el0_session();
        // SAFETY: prepared AS; `enter_step` masks EL1 IRQs as `el0::enter`
        // requires; the session belongs to the running task, which the call
        // checks rather than assumes.
        let mut event = enter_step(session, root, entry, sp, before_enter);
        let mut stats = SessionStats::default();
        {
            loop {
                match event {
                    el0::El0Outcome::Svc { imm } => match syscall::decode(imm) {
                        Syscall::Ping => {
                            stats.pings = stats.pings.saturating_add(1);
                            // SAFETY: the outcome being matched is `Svc`, which
                            // is resumable by definition; IRQs are still masked
                            // by the enclosing `without_irqs`.
                            event = resume_step(session);
                        }
                        Syscall::Send => {
                            let msg = ipc::Message {
                                tag: el0::saved_gpr(session, 1) as u32,
                                a: el0::saved_gpr(session, 2),
                                b: el0::saved_gpr(session, 3),
                            };
                            let status =
                                match ipc::send_from_slot(el0::saved_gpr(session, 0) as usize, msg)
                                {
                                    Ok(()) => {
                                        stats.sends = stats.sends.saturating_add(1);
                                        Status::Ok
                                    }
                                    Err(ipc::SendError::Full) => Status::Full,
                                    Err(_) => {
                                        stats.authority_refusals =
                                            stats.authority_refusals.saturating_add(1);
                                        Status::Authority
                                    }
                                };
                            el0::set_saved_gpr(session, 0, status.as_u64());
                            // SAFETY: as `Ping` — a resumable `Svc` outcome with
                            // IRQs masked. The reply is already in the saved
                            // register file, so the resume delivers it.
                            event = resume_step(session);
                        }
                        Syscall::Recv => {
                            // The call this whole restructuring exists for
                            // (ADR-0022 §1). `recv_from_slot` parks the task on
                            // an empty mailbox — a switch, which is why it must
                            // not run under a mask this loop is holding, and
                            // does not: every masked region here is one step.
                            //
                            // The agent's text never sees the wait. It resumes
                            // with `Ok` and the payload, whenever it next runs.
                            let slot = el0::saved_gpr(session, 0) as usize;
                            let status = recv_reply(session, ipc::recv_from_slot(slot), &mut stats);
                            el0::set_saved_gpr(session, 0, status.as_u64());
                            // SAFETY: as `Send`.
                            event = resume_step(session);
                        }
                        Syscall::TryRecv => {
                            // The non-blocking half (ADR-0022 §4), and the only
                            // producer of `Status::Empty` left in the kernel.
                            let slot = el0::saved_gpr(session, 0) as usize;
                            let status =
                                recv_reply(session, ipc::try_recv_from_slot(slot), &mut stats);
                            el0::set_saved_gpr(session, 0, status.as_u64());
                            // SAFETY: as `Send`.
                            event = resume_step(session);
                        }
                        Syscall::Exit => {
                            // SAFETY: the session is resumable but will not be
                            // resumed — this returns, and EL0 is unreachable
                            // without another `enter`. Ending it here is what
                            // clears `el0_kernel_ttbr0` while nothing can fault.
                            end_step(session);
                            stats.end = SessionEnd::Exit;
                            return Ok(stats);
                        }
                        Syscall::Unknown { imm } => {
                            // SAFETY: as `Exit`. An SVC this kernel does not
                            // implement ends the session rather than inventing
                            // a behaviour for it.
                            end_step(session);
                            stats.end = SessionEnd::UnknownSvc { imm };
                            return Ok(stats);
                        }
                    },
                    el0::El0Outcome::Irq => {
                        irq::handle_cpu_irq();
                        stats.irqs = stats.irqs.saturating_add(1);
                        // SAFETY: `Irq` is resumable, and the handler above has
                        // run to completion. `ELR` still points at the
                        // interrupted instruction, which the architecture
                        // re-executes — resuming here does not skip it.
                        event = resume_step(session);
                    }
                    el0::El0Outcome::DataAbort { esr, far }
                    | el0::El0Outcome::OtherSync { esr, far } => {
                        // Mechanism, unconditional: an EL0 context that took a
                        // synchronous exception has no defined continuation,
                        // and the kernel is the only party positioned to know
                        // it. What happens to the *task* is not decided here
                        // (ADR-0018 §1–2).
                        note_fault();
                        // SAFETY: these are the outcomes that already ended the
                        // session — `EL0_CAN_RESUME` is clear and the vector
                        // path has released the kernel root. This makes it
                        // explicit rather than relying on it.
                        end_step(session);
                        stats.end = SessionEnd::Fault { esr, far };
                        return Ok(stats);
                    }
                }
            }
        }
    }

    /// Tear down the AS (revokes user + any device leaves; returns pool frames).
    pub fn destroy(self) {
        self.aspace.destroy();
    }
}

/// Dispatch a returned `SVC` outcome for demo agents.
pub fn report_svc(prefix: &str, outcome: el0::El0Outcome) {
    match outcome {
        el0::El0Outcome::Svc { imm } => match syscall::decode(imm) {
            Syscall::Ping => crate::kprintln!("{prefix}: svc ping"),
            Syscall::Exit => crate::kprintln!("{prefix}: svc exit"),
            Syscall::Send => crate::kprintln!("{prefix}: svc send"),
            Syscall::Recv => crate::kprintln!("{prefix}: svc recv"),
            Syscall::TryRecv => crate::kprintln!("{prefix}: svc try-recv"),
            Syscall::Unknown { imm } => crate::kprintln!("{prefix}: svc refuse imm={imm:#x}"),
        },
        other => crate::kprintln!("{prefix}: unexpected {other:?}"),
    }
}

// --- Concurrent two-agent barrier (cooperative) ---

/// bit0 = alpha prepared, bit1 = beta prepared,
/// bit2 = alpha el0 done, bit3 = beta el0 done,
/// bit4 = alpha destroyed, bit5 = beta destroyed.
static CONC: AtomicU32 = AtomicU32::new(0);

const A_PREP: u32 = 1;
const B_PREP: u32 = 2;
const A_EL0: u32 = 4;
const B_EL0: u32 = 8;
const A_DIE: u32 = 16;
const B_DIE: u32 = 32;

fn wait_bits(need: u32) {
    while CONC.load(Ordering::Acquire) & need != need {
        sched::yield_now();
    }
}

/// Free-count while both peers hold a prepared AS (set by alpha after barrier).
static FREE_AT_DUAL_LIVE: AtomicU32 = AtomicU32::new(0);

/// Peer A: prepare → wait peer → EL0 ping → wait peer → destroy.
pub fn concurrent_agent_alpha() {
    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("agent-a: create FAILED {e:?}");
            return;
        }
    };
    CONC.fetch_or(A_PREP, Ordering::Release);
    wait_bits(A_PREP | B_PREP);

    let free_live = mm::frames::free_count();
    FREE_AT_DUAL_LIVE.store(free_live, Ordering::Release);

    match agent.run_user_prog(&kernel_core::prog::encode_svc_imm(0)) {
        Ok(out) => report_svc("agent-a", out),
        Err(e) => crate::kprintln!("agent-a: el0 FAILED {e:?}"),
    }
    CONC.fetch_or(A_EL0, Ordering::Release);
    wait_bits(A_EL0 | B_EL0);

    agent.destroy();
    CONC.fetch_or(A_DIE, Ordering::Release);
    wait_bits(A_DIE | B_DIE);

    let free_after = mm::frames::free_count();
    let free_live = FREE_AT_DUAL_LIVE.load(Ordering::Acquire);
    if free_after > free_live {
        crate::kprintln!("agents: concurrent ok  pool={free_after}");
    } else {
        crate::kprintln!("agents: concurrent LEAK live={free_live} after={free_after}");
    }
}

/// Peer B: same protocol as alpha (symmetric barrier).
pub fn concurrent_agent_beta() {
    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("agent-b: create FAILED {e:?}");
            return;
        }
    };
    CONC.fetch_or(B_PREP, Ordering::Release);
    wait_bits(A_PREP | B_PREP);

    match agent.run_user_prog(&kernel_core::prog::encode_svc_imm(0)) {
        Ok(out) => report_svc("agent-b", out),
        Err(e) => crate::kprintln!("agent-b: el0 FAILED {e:?}"),
    }
    CONC.fetch_or(B_EL0, Ordering::Release);
    wait_bits(A_EL0 | B_EL0);

    agent.destroy();
    CONC.fetch_or(B_DIE, Ordering::Release);
    sched::yield_now();
}
