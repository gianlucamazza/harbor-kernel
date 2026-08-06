//! Cooperative agent shell (post-M5 productization).
//!
//! An **agent** is an EL1 scheduled body that owns an [`AddressSpace`] and may
//! enter EL0 through [`crate::arch::el0`] sessions. Matches ADR-0006
//! (cooperative) and ADR-0014 (kernel `TTBR0` on lower-EL return).
//!
//! Sessions support **SVC resume** and **IRQ resume** (architectural):
//! - `svc #0` ([`kernel_core::syscall::SYS_PING`]) — count and resume
//! - `svc #1` ([`kernel_core::syscall::SYS_EXIT`]) — end session
//! - `svc #2` ([`kernel_core::syscall::SYS_PUTC`]) — TX low 8 bits of saved `x0`
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
//! The loop still runs inside `cpu::without_irqs`, which is now about EL1
//! interrupt timing rather than about protecting a shared slot.

use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::a64;
use kernel_core::syscall::{self, Syscall};

use crate::arch::{cpu, el0};
use crate::console;
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

/// Counters from one multi-event EL0 session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionStats {
    pub pings: u32,
    pub putcs: u32,
    pub irqs: u32,
    /// Why the session stopped. See [`SessionEnd`].
    pub end: SessionEnd,
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

    /// Multi-event session: resume after Ping / Putc / Irq; stop on Exit / fault.
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
    /// never observed as [`el0::El0Outcome::Irq`].
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
        cpu::without_irqs(|| {
            before_enter();
            // The session this task owns. Every call below passes it, and
            // `arch::el0` refuses to act on a session the assembly would not
            // see — a switch that stopped publishing panics here instead of
            // handing this loop another task's saved registers (ADR-0017 §1).
            let session = sched::current_el0_session();
            // SAFETY: prepared AS; IRQs masked at EL1 around enter/resume; the
            // session belongs to the running task, which is what the call
            // checks rather than assumes.
            let mut event = unsafe { el0::enter(session, root, entry, sp) };
            let mut stats = SessionStats::default();
            loop {
                match event {
                    el0::El0Outcome::Svc { imm } => match syscall::decode(imm) {
                        Syscall::Ping => {
                            stats.pings = stats.pings.saturating_add(1);
                            // SAFETY: the outcome being matched is `Svc`, which
                            // is resumable by definition; IRQs are still masked
                            // by the enclosing `without_irqs`.
                            event = unsafe { el0::resume(session) };
                        }
                        Syscall::Putc => {
                            let byte = (el0::saved_x0(session) & 0xFF) as u8;
                            let _ = console::with_tx(|uart| uart.write_byte(byte));
                            stats.putcs = stats.putcs.saturating_add(1);
                            // SAFETY: as `Ping` — a resumable `Svc` outcome
                            // with IRQs masked. Note the console write above is
                            // granted to any agent with no capability check;
                            // that is ADR-0017 §3, and slice 3 of M7, not this
                            // block's.
                            event = unsafe { el0::resume(session) };
                        }
                        Syscall::Exit => {
                            // SAFETY: the session is resumable but will not be
                            // resumed — this returns, and EL0 is unreachable
                            // without another `enter`. Ending it here is what
                            // clears `el0_kernel_ttbr0` while nothing can fault.
                            unsafe { el0::end_session(session) };
                            stats.end = SessionEnd::Exit;
                            return Ok(stats);
                        }
                        Syscall::Unknown { imm } => {
                            // SAFETY: as `Exit`. An SVC this kernel does not
                            // implement ends the session rather than inventing
                            // a behaviour for it.
                            unsafe { el0::end_session(session) };
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
                        event = unsafe { el0::resume(session) };
                    }
                    el0::El0Outcome::DataAbort { esr, far }
                    | el0::El0Outcome::OtherSync { esr, far } => {
                        // SAFETY: these are the outcomes that already ended the
                        // session — `EL0_CAN_RESUME` is clear and the vector
                        // path has released the kernel root. This makes it
                        // explicit rather than relying on it.
                        unsafe { el0::end_session(session) };
                        stats.end = SessionEnd::Fault { esr, far };
                        return Ok(stats);
                    }
                }
            }
        })
    }

    /// Tear down the AS (revokes user + any device leaves; returns pool frames).
    pub fn destroy(self) {
        self.aspace.destroy();
    }
}

#[inline]
fn push_word(out: &mut [u8], at: &mut usize, word: u32) {
    let b = a64::le_bytes(word);
    out[*at..*at + 4].copy_from_slice(&b);
    *at += 4;
}

/// A64: `svc #imm` ; `b .`
pub fn encode_svc_imm(imm: u16) -> [u8; 8] {
    let mut out = [0u8; 8];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::svc(imm));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// Dispatch a returned `SVC` outcome for demo agents.
pub fn report_svc(prefix: &str, outcome: el0::El0Outcome) {
    match outcome {
        el0::El0Outcome::Svc { imm } => match syscall::decode(imm) {
            Syscall::Ping => crate::kprintln!("{prefix}: svc ping"),
            Syscall::Exit => crate::kprintln!("{prefix}: svc exit"),
            Syscall::Putc => crate::kprintln!("{prefix}: svc putc"),
            Syscall::Unknown { imm } => crate::kprintln!("{prefix}: svc refuse imm={imm:#x}"),
        },
        other => crate::kprintln!("{prefix}: unexpected {other:?}"),
    }
}

/// A64: `svc #0; svc #0; svc #1; b .` — two pings then exit (resume path).
pub fn encode_ping_ping_exit() -> [u8; 16] {
    let mut out = [0u8; 16];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::svc(0));
    push_word(&mut out, &mut i, a64::svc(0));
    push_word(&mut out, &mut i, a64::svc(1));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// A64: `movz x0, #'H'; svc #2; movz x0, #'!'; svc #2; svc #1; b .`
pub fn encode_putc_hi_exit() -> [u8; 24] {
    let mut out = [0u8; 24];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, u16::from(b'H')));
    push_word(&mut out, &mut i, a64::svc(2));
    push_word(&mut out, &mut i, a64::movz_x(0, u16::from(b'!')));
    push_word(&mut out, &mut i, a64::svc(2));
    push_word(&mut out, &mut i, a64::svc(1));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// Finite spin then `SYS_EXIT` — GPRs survive IRQ save/restore, so this makes
/// forward progress under plain (architectural) IRQ resume.
///
/// ```text
/// movz x0, #iters          // low 16 bits; high half zero
/// 1: sub  x0, x0, #1
///    cbnz x0, 1b            // offset −1 word from the cbnz (gas-checked)
/// svc #1
/// b .
/// ```
///
/// Pair with [`el0::set_entry_irqs_unmasked`] and
/// [`crate::arch::timer::accelerate_next_tick`] so a timer IRQ arrives while
/// the counter is still non-zero.
pub fn encode_spin_exit(iters: u16) -> [u8; 20] {
    let mut out = [0u8; 20];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x(0, iters));
    push_word(&mut out, &mut i, a64::sub_x_imm(0, 0, 1));
    push_word(&mut out, &mut i, a64::cbnz_x(0, -1));
    push_word(&mut out, &mut i, a64::svc(1));
    push_word(&mut out, &mut i, a64::b_self());
    out
}

/// PL011 RX poll once at `USER_PL011_VA` (`0x5000_0000`):
/// if RX not empty, `SYS_PUTC` the byte; always `SYS_EXIT`.
///
/// Empty FIFO → zero putcs (honest “no data” path). A pending character → one
/// putc. Does not invent receive data.
pub fn encode_pl011_rx_poll_exit() -> [u8; 28] {
    let mut out = [0u8; 28];
    let mut i = 0;
    push_word(&mut out, &mut i, a64::movz_x_lsl16(0, 0x5000));
    push_word(&mut out, &mut i, a64::ldr_w_imm(1, 0, 0x18));
    // RXFE (bit 4) set → empty → skip ldrb + putc
    push_word(&mut out, &mut i, a64::tbnz_w(1, 4, 3));
    push_word(&mut out, &mut i, a64::ldrb_w(0, 0));
    push_word(&mut out, &mut i, a64::svc(2));
    push_word(&mut out, &mut i, a64::svc(1));
    push_word(&mut out, &mut i, a64::b_self());
    out
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

    match agent.run_user_prog(&encode_svc_imm(0)) {
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

    match agent.run_user_prog(&encode_svc_imm(0)) {
        Ok(out) => report_svc("agent-b", out),
        Err(e) => crate::kprintln!("agent-b: el0 FAILED {e:?}"),
    }
    CONC.fetch_or(B_EL0, Ordering::Release);
    wait_bits(A_EL0 | B_EL0);

    agent.destroy();
    CONC.fetch_or(B_DIE, Ordering::Release);
    sched::yield_now();
}
