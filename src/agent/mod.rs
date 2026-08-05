//! Cooperative agent shell (post-M5 productization).
//!
//! An **agent** is an EL1 scheduled body that owns an [`AddressSpace`] and may
//! enter EL0 through [`crate::arch::el0`] sessions. Matches ADR-0006
//! (cooperative) and ADR-0014 (kernel `TTBR0` on lower-EL return).
//!
//! Sessions support **SVC resume** and **IRQ resume**:
//! - `svc #0` ([`kernel_core::syscall::SYS_PING`]) — count and resume
//! - `svc #1` ([`kernel_core::syscall::SYS_EXIT`]) — end session
//! - `svc #2` ([`kernel_core::syscall::SYS_PUTC`]) — TX low 8 bits of saved `x0`
//! - [`el0::El0Outcome::Irq`] — run [`crate::irq::handle_cpu_irq`], then resume
//!
//! Default entry masks IRQs in EL0. Call [`el0::set_entry_irqs_unmasked`] before
//! a session that should take lower-EL IRQs (e.g. timer while user waits).

use core::sync::atomic::{AtomicU32, Ordering};

use crate::arch::{cpu, el0};
use crate::console;
use crate::irq;
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

/// Counters from one multi-event EL0 session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionStats {
    pub pings: u32,
    pub putcs: u32,
    pub irqs: u32,
}

/// How [`Agent::run_session`] treats [`el0::El0Outcome::Irq`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IrqResume {
    /// `handle_cpu_irq` then [`el0::resume`] (re-execute interrupted insn).
    Plain,
    /// First IRQ: handle, skip one insn, mask EL0 IRQs, resume (WFI wake).
    WakeSkipInsn,
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

    /// Multi-event session: resume after Ping / Putc / Irq; stop on Exit / fault.
    pub fn run_user_prog_resuming(&mut self, prog: &[u8]) -> Result<SessionStats, AgentError> {
        self.run_session(prog, IrqResume::Plain)
    }

    /// Like [`run_user_prog_resuming`], but the first IRQ is treated as a wake:
    /// handle the IRQ, skip one user insn (typically `WFI`), mask EL0 IRQs, resume.
    ///
    /// Use with [`encode_wfi_exit`] so the session reaches `SYS_EXIT` after one tick.
    pub fn run_user_prog_irq_wake(&mut self, prog: &[u8]) -> Result<SessionStats, AgentError> {
        self.run_session(prog, IrqResume::WakeSkipInsn)
    }

    fn run_session(
        &mut self,
        prog: &[u8],
        irq_policy: IrqResume,
    ) -> Result<SessionStats, AgentError> {
        self.aspace
            .poke_user(0, prog)
            .map_err(|_| AgentError::Poke)?;
        let root = self.aspace.root_phys();
        let entry = self.aspace.user_entry_va();
        let sp = self.aspace.user_sp();
        cpu::without_irqs(|| {
            // SAFETY: prepared AS; sole session; IRQs masked at EL1 around enter/resume.
            let mut event = unsafe { el0::enter(root, entry, sp) };
            let mut stats = SessionStats::default();
            loop {
                match event {
                    el0::El0Outcome::Svc { imm } => match syscall::decode(imm) {
                        Syscall::Ping => {
                            stats.pings = stats.pings.saturating_add(1);
                            event = unsafe { el0::resume() };
                        }
                        Syscall::Putc => {
                            let byte = (el0::saved_x0() & 0xFF) as u8;
                            let _ = console::with_tx(|uart| uart.write_byte(byte));
                            stats.putcs = stats.putcs.saturating_add(1);
                            event = unsafe { el0::resume() };
                        }
                        Syscall::Exit => {
                            el0::end_session();
                            return Ok(stats);
                        }
                        Syscall::Unknown { .. } => {
                            el0::end_session();
                            return Ok(stats);
                        }
                    },
                    el0::El0Outcome::Irq => {
                        // Same path as same-EL IRQ: claim → dispatch → EOI.
                        irq::handle_cpu_irq();
                        stats.irqs = stats.irqs.saturating_add(1);
                        if matches!(irq_policy, IrqResume::WakeSkipInsn) && stats.irqs == 1 {
                            // Skip WFI; mask further EL0 IRQs so exit SVC runs.
                            unsafe { el0::advance_saved_elr(4) };
                            el0::mask_saved_irqs();
                        }
                        event = unsafe { el0::resume() };
                    }
                    _ => {
                        el0::end_session();
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

/// A64: `svc #imm` ; `b .`
pub fn encode_svc_imm(imm: u16) -> [u8; 8] {
    let word = 0xD400_0001u32 | ((imm as u32) << 5);
    let b = word.to_le_bytes();
    [b[0], b[1], b[2], b[3], 0x00, 0x00, 0x00, 0x14]
}

/// A64: `movz x0, #imm16` (LSL #0).
#[inline]
pub fn encode_movz_x0(imm16: u16) -> [u8; 4] {
    let word = 0xD280_0000u32 | ((imm16 as u32) << 5);
    word.to_le_bytes()
}

/// A64: `svc #imm` only (no trailing branch).
#[inline]
pub fn encode_svc_word(imm: u16) -> [u8; 4] {
    let word = 0xD400_0001u32 | ((imm as u32) << 5);
    word.to_le_bytes()
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
    let s0 = encode_svc_word(0);
    let s1 = encode_svc_word(1);
    [
        s0[0], s0[1], s0[2], s0[3], // svc #0
        s0[0], s0[1], s0[2], s0[3], // svc #0
        s1[0], s1[1], s1[2], s1[3], // svc #1
        0x00, 0x00, 0x00, 0x14, // b .
    ]
}

/// A64: `movz x0, #'H'; svc #2; movz x0, #'!'; svc #2; svc #1; b .`
pub fn encode_putc_hi_exit() -> [u8; 24] {
    let m_h = encode_movz_x0(u16::from(b'H'));
    let m_b = encode_movz_x0(u16::from(b'!'));
    let putc = encode_svc_word(2);
    let exit = encode_svc_word(1);
    [
        m_h[0], m_h[1], m_h[2], m_h[3],
        putc[0], putc[1], putc[2], putc[3],
        m_b[0], m_b[1], m_b[2], m_b[3],
        putc[0], putc[1], putc[2], putc[3],
        exit[0], exit[1], exit[2], exit[3],
        0x00, 0x00, 0x00, 0x14,
    ]
}

/// A64: `b .; svc #1; b .` — tight wait for IRQ, then exit.
///
/// Pair with [`Agent::run_user_prog_irq_wake`] + [`el0::set_entry_irqs_unmasked`]:
/// the first IRQ skips the branch and runs `SYS_EXIT`. Prefer this over EL0
/// `WFI` (QEMU often falls through) and over long counted spins (starve the
/// cooperative scheduler under TCG).
pub fn encode_branch_wait_exit() -> [u8; 12] {
    let exit = encode_svc_word(1);
    [
        0x00, 0x00, 0x00, 0x14, // b .
        exit[0], exit[1], exit[2], exit[3],
        0x00, 0x00, 0x00, 0x14, // b .
    ]
}

/// PL011 RX poll once at `USER_PL011_VA`: if RX not empty, `SYS_PUTC` the byte; then exit.
///
/// ```text
/// movz x0, #0x5000, lsl #16     // USER_PL011_VA
/// ldr  w1, [x0, #0x18]          // FR
/// tbnz w1, #4, 1f               // RXFE set → empty
/// ldrb w0, [x0]                 // DR
/// svc  #2                       // putc
/// 1: svc #1                     // exit
/// b .
/// ```
pub fn encode_pl011_rx_poll_exit() -> [u8; 28] {
    // movz x0, #0x5000, lsl #16
    let movz: u32 = 0xD2A0_0000 | (0x5000u32 << 5);
    let movz_b = movz.to_le_bytes();
    // ldr w1, [x0, #0x18]  — Rn=x0, Rt=w1, imm12=6 (byte offset 0x18)
    let ldr_fr: u32 = 0xB940_0000 | (6u32 << 10) | 1;
    let ldr_fr_b = ldr_fr.to_le_bytes();
    // tbnz w1, #4, +3 words → svc #1 when RXFE set (empty).
    // PC-relative word offset from this insn; skip ldrb + svc #2.
    let tbnz: u32 = 0x3700_0000 | (4u32 << 19) | ((3u32 & 0x3FFF) << 5) | 1;
    let tbnz_b = tbnz.to_le_bytes();
    // ldrb w0, [x0]  — Rn=x0, Rt=w0
    let ldrb_b = 0x3940_0000u32.to_le_bytes();
    let putc = encode_svc_word(2);
    let exit = encode_svc_word(1);
    [
        movz_b[0], movz_b[1], movz_b[2], movz_b[3],
        ldr_fr_b[0], ldr_fr_b[1], ldr_fr_b[2], ldr_fr_b[3],
        tbnz_b[0], tbnz_b[1], tbnz_b[2], tbnz_b[3],
        ldrb_b[0], ldrb_b[1], ldrb_b[2], ldrb_b[3],
        putc[0], putc[1], putc[2], putc[3],
        exit[0], exit[1], exit[2], exit[3],
        0x00, 0x00, 0x00, 0x14,
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
