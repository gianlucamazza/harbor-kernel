//! Cooperative agent shell (post-M5 productization).
//!
//! An **agent** is an EL1 scheduled body that owns an [`AddressSpace`] and may
//! enter EL0 through [`crate::arch::el0`] sessions (IRQs masked). Matches
//! ADR-0006 (cooperative) and ADR-0014 (kernel `TTBR0` on lower-EL return).
//!
//! Sessions support **SVC resume**: after `svc #0` (ping) the same user context
//! continues at `ELR+4`. `svc #1` ([`kernel_core::syscall::SYS_EXIT`]) ends the
//! session. Faults end the session without resume.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::arch::{cpu, el0};
use crate::mm::{self, AddressSpace, AsError};
use crate::sched;
use kernel_core::syscall::{self, Syscall};

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
        let outcome = cpu::without_irqs(|| unsafe {
            el0::run(
                self.aspace.root_phys(),
                self.aspace.user_entry_va(),
                self.aspace.user_sp(),
            )
        });
        Ok(outcome)
    }

    /// Multi-SVC session: resume after Ping, stop on Exit / fault / refuse.
    ///
    /// Returns the number of successful Ping syscalls handled.
    pub fn run_user_prog_resuming(&mut self, prog: &[u8]) -> Result<u32, AgentError> {
        self.aspace
            .poke_user(0, prog)
            .map_err(|_| AgentError::Poke)?;
        let root = self.aspace.root_phys();
        let entry = self.aspace.user_entry_va();
        let sp = self.aspace.user_sp();
        cpu::without_irqs(|| {
            // SAFETY: prepared AS; sole session; IRQs masked.
            let mut event = unsafe { el0::enter(root, entry, sp) };
            let mut pings = 0u32;
            loop {
                match event {
                    el0::El0Outcome::Svc { imm } => match syscall::decode(imm) {
                        Syscall::Ping => {
                            pings = pings.saturating_add(1);
                            event = unsafe { el0::resume() };
                        }
                        Syscall::Exit => {
                            el0::end_session();
                            return Ok(pings);
                        }
                        Syscall::Unknown { .. } => {
                            el0::end_session();
                            return Ok(pings);
                        }
                    },
                    _ => {
                        el0::end_session();
                        return Ok(pings);
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

/// A64: `svc #imm` ; `b .`
pub fn encode_svc_imm(imm: u16) -> [u8; 8] {
    let word = 0xD400_0001u32 | ((imm as u32) << 5);
    let b = word.to_le_bytes();
    [b[0], b[1], b[2], b[3], 0x00, 0x00, 0x00, 0x14]
}

/// Dispatch a returned `SVC` outcome for demo agents.
pub fn report_svc(prefix: &str, outcome: el0::El0Outcome) {
    match outcome {
        el0::El0Outcome::Svc { imm } => match syscall::decode(imm) {
            Syscall::Ping => crate::kprintln!("{prefix}: svc ping"),
            Syscall::Exit => crate::kprintln!("{prefix}: svc exit"),
            Syscall::Unknown { imm } => crate::kprintln!("{prefix}: svc refuse imm={imm:#x}"),
        },
        other => crate::kprintln!("{prefix}: unexpected {other:?}"),
    }
}

/// A64: `svc #0; svc #0; svc #1; b .` — two pings then exit (resume path).
pub fn encode_ping_ping_exit() -> [u8; 16] {
    let s0 = encode_svc_imm(0);
    let s1 = encode_svc_imm(1);
    [
        s0[0], s0[1], s0[2], s0[3], // svc #0
        s0[0], s0[1], s0[2], s0[3], // svc #0
        s1[0], s1[1], s1[2], s1[3], // svc #1
        0x00, 0x00, 0x00, 0x14, // b .
    ]
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

    // Both AS live — free count is the dual-live baseline (ignore other tasks).
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
    // Both AS gone ⇒ pool free must rise strictly above the dual-live mark.
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
    // Alpha owns the pool check; yield so it can observe post-destroy free.
    sched::yield_now();
}
