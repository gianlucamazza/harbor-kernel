//! EL0 entry / SVC resume (ADR-0014).
//!
//! ## Protocol
//!
//! 1. [`enter`] publishes the kernel root, switches to the user `TTBR0`,
//!    programs `ELR`/`SPSR`/`SP_EL0`, `ERET` to EL0.
//! 2. Lower-EL sync: `kernel_entry` → `switch_ttbr0(kernel)` → classify. On
//!    **SVC**, user GPRs/`ELR`/`SPSR`/`SP_EL0` are saved. AArch64 already sets
//!    `ELR` to the instruction *after* the SVC (preferred return) — no software
//!    `+4`. [`resume`] continues that context.
//! 3. [`resume`] re-installs the user root and `ERET`s with the saved state.
//! 4. [`end_session`] clears the published kernel root.
//!
//! [`run`] is one-shot: `enter` + [`end_session`]. Default entry masks IRQs in
//! EL0 (`SPSR` DAIF.I); sessions that need timer/UART while user runs call
//! [`set_entry_irqs_unmasked`] before [`enter`].

//! ## Why `static mut` here, when `sync.rs` argues it is unacceptable
//!
//! [`crate::sync`] exists because a `static mut` in edition 2024 has no way to
//! state who may touch it. Every global below is one anyway, and the reason is
//! not laziness: the assembly in this file reaches them **by symbol name**
//! (`adrp`/`add` against `EL0_SAVED`, `el0_kernel_ttbr0`, `EL0_ENTRY_SPSR`), and
//! so does `vectors.s`. A `SyncCell` has no linker-visible name to load, and an
//! `UnsafeCell` wrapper would only move the same raw access one layer down while
//! making the offsets that `el0_resume` hard-codes depend on a layout Rust does
//! not promise.
//!
//! What replaces the missing type-level protection is the session contract
//! below. It is a contract in prose, which is weaker, and the module says so.
//!
//! ## Session contract
//!
//! One session at a time, for the whole machine: every global here is a single
//! slot. A session is live from [`enter`] until an outcome that is not
//! resumable, or until [`end_session`]. While it is live:
//!
//! - `el0_kernel_ttbr0` is non-zero, and `vectors.s` **requires** that — a
//!   lower-EL exception with it clear reaches [`el0_missing_kernel_ttbr`] and
//!   panics. This is why [`end_session`] is `unsafe`.
//! - the caller must not `yield_now`. Nothing enforces it; the whole loop in
//!   [`crate::agent`] runs inside `cpu::without_irqs`, which is what makes the
//!   single slot hold today. A yield inside a session would let a second agent
//!   overwrite the first one's saved context, and `el0_run_sp` would restore
//!   the wrong stack.
//!
//! Both of those are the reason [`crate::agent`] can only enter EL0 one agent
//! at a time, whatever the scheduler is doing. Moving this state into the TCB
//! is the named successor, not a refactor to slip in unannounced.

use crate::arch::exception::{TrapFrame, read_esr_el1, read_far_el1};
use crate::arch::mmu;

/// `SPSR_EL1` for EL0t with DAIF all masked (default session contract).
const SPSR_EL0_IRQS_MASKED: u64 = 0x3c0;
/// `SPSR_EL1` for EL0t with DAIF.I clear — IRQs may take lower-EL IRQ vectors.
const SPSR_EL0_IRQS_OPEN: u64 = 0x340;

/// Result of one EL0 stretch (enter or resume until the next lower-EL sync).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum El0Outcome {
    /// `SVC` from AArch64 EL0. Session may [`resume`].
    Svc { imm: u16 },
    /// Data abort from lower EL. Session ends.
    DataAbort { esr: u64, far: u64 },
    /// IRQ while EL0 ran with IRQs unmasked. Session may [`resume`] after handle.
    Irq,
    /// Other sync from lower EL. Session ends.
    OtherSync { esr: u64, far: u64 },
}

/// Next [`enter`] uses EL0 `SPSR` with IRQs masked (default after boot / end).
#[inline]
pub fn set_entry_irqs_masked() {
    // SAFETY: single core, and `EL0_ENTRY_SPSR` is read by `el0_run` only, at
    // the start of a session — never by `el0_resume`, so this cannot alter a
    // session already under way. Safe rather than `unsafe fn` for that reason:
    // the worst a mistimed call does is choose the mask for the *next* entry.
    unsafe { EL0_ENTRY_SPSR = SPSR_EL0_IRQS_MASKED };
}

/// Next [`enter`] uses EL0 `SPSR` with DAIF.I clear (IRQ → [`El0Outcome::Irq`]).
#[inline]
pub fn set_entry_irqs_unmasked() {
    // SAFETY: as [`set_entry_irqs_masked`] — read by `el0_run` at entry only.
    unsafe { EL0_ENTRY_SPSR = SPSR_EL0_IRQS_OPEN };
}

/// One-shot: enter EL0 until the first sync, then end the session.
///
/// # Safety
/// Same as [`enter`].
pub unsafe fn run(ttbr0_phys: usize, entry: u64, user_sp: u64) -> El0Outcome {
    // SAFETY: `enter`'s obligations are the caller's, forwarded by this
    // function's own `# Safety`. The `end_session` that follows is sound
    // because `enter` has returned: whatever the outcome, this session took its
    // one event and is not going to be resumed — that is what "one-shot" means.
    let outcome = unsafe { enter(ttbr0_phys, entry, user_sp) };
    // SAFETY: `enter` has returned, so this session took its one event and will
    // not be resumed — which is what makes ending it here sound.
    unsafe { end_session() };
    outcome
}

/// Enter EL0 until the first lower-EL sync.
///
/// # Safety
/// Prepared user root; IRQs masked; sole session.
pub unsafe fn enter(ttbr0_phys: usize, entry: u64, user_sp: u64) -> El0Outcome {
    let Some(kernel_ttbr) = mmu::kernel_root_phys() else {
        panic!("el0::enter: kernel map not activated");
    };
    // SAFETY: single core with IRQs masked (the caller's obligation), so no
    // other context can be between these writes and the `el0_run` that reads
    // them. `EL0_CAN_RESUME` is cleared *before* the entry so that a fault on
    // the very first instruction cannot be mistaken for a resumable event left
    // over from a previous session.
    unsafe {
        EL0_USER_TTBR = ttbr0_phys as u64;
        EL0_CAN_RESUME = 0;
        unpack(el0_run(
            ttbr0_phys as u64,
            entry,
            user_sp,
            kernel_ttbr as u64,
        ))
    }
}

/// Continue after [`El0Outcome::Svc`] or [`El0Outcome::Irq`].
///
/// After SVC, `ELR` already points past the insn. After IRQ, `ELR` is the
/// interrupted insn — architectural re-execute on resume (no software skip).
///
/// # Safety
/// Prior event was resumable; IRQs masked at EL1; session not ended.
pub unsafe fn resume() -> El0Outcome {
    // SAFETY: reads of single-slot session state on one core with IRQs masked.
    // Both are checked rather than assumed: `el0_resume` would `eret` into a
    // context that was never saved, and `vectors.s` would take the next
    // lower-EL exception with no kernel root to reinstall. Panicking here is
    // the difference between a message and an unrecoverable fetch.
    if unsafe { EL0_CAN_RESUME } == 0 {
        panic!("el0::resume: no resumable session");
    }
    // SAFETY: as above.
    if unsafe { el0_kernel_ttbr0 } == 0 {
        panic!("el0::resume: session kernel TTBR cleared");
    }
    // SAFETY: the two checks above are exactly `el0_resume`'s preconditions.
    unsafe { unpack(el0_resume()) }
}

/// Clear session symbols (call after the last event if not already cleared).
///
/// # Safety
/// No EL0 session may be live. This clears `el0_kernel_ttbr0`, which
/// `vectors.s` requires to be non-zero on every lower-EL exception: calling it
/// while EL0 can still be entered turns the next fault from that agent into
/// [`el0_missing_kernel_ttbr`] and a panic. It was a safe `fn`, which made
/// breaking the vector path's precondition ordinary safe Rust.
#[inline]
pub unsafe fn end_session() {
    // SAFETY: single-slot session state, one core; the caller has established
    // that no session is live, so nothing is going to read these again before
    // the next `enter` writes them.
    unsafe {
        el0_kernel_ttbr0 = 0;
        EL0_CAN_RESUME = 0;
        EL0_USER_TTBR = 0;
        EL0_ENTRY_SPSR = SPSR_EL0_IRQS_MASKED;
    }
}

fn unpack(packed: u64) -> El0Outcome {
    let kind = (packed & 0xFFFF_FFFF) as u32;
    match kind {
        1 => El0Outcome::Svc {
            imm: (packed >> 32) as u16,
        },
        2 => {
            let (esr, far) = fault_syndrome();
            El0Outcome::DataAbort { esr, far }
        }
        4 => El0Outcome::Irq,
        _ => {
            let (esr, far) = fault_syndrome();
            El0Outcome::OtherSync { esr, far }
        }
    }
}

/// The syndrome saved by the vector path for the event being decoded.
fn fault_syndrome() -> (u64, u64) {
    // SAFETY: `exception_sync_el0` wrote both on the way here, from the very
    // event whose packed kind is being decoded — the vector path runs to
    // completion before `el0_run_finish` returns that value. Single slot, one
    // core, and nothing else writes them until the next lower-EL exception.
    unsafe { (EL0_ESR, EL0_FAR) }
}

/// Low 64 bits of user `x0` at the last SVC/IRQ (for `SYS_PUTC`, etc.).
#[inline]
pub fn saved_x0() -> u64 {
    // SAFETY: a plain read of one word of single-slot session state on one
    // core. Safe rather than `unsafe fn` because a mistimed call returns a
    // stale value rather than breaking an invariant — the caller is expected to
    // have just received `Svc` or `Irq`, and nothing else consults this.
    unsafe { EL0_SAVED.gpr[0] }
}

#[unsafe(no_mangle)]
static mut EL0_ESR: u64 = 0;
#[unsafe(no_mangle)]
static mut EL0_FAR: u64 = 0;
#[unsafe(no_mangle)]
static mut EL0_KIND: u64 = 0;
#[unsafe(no_mangle)]
static mut EL0_CAN_RESUME: u64 = 0;

#[unsafe(no_mangle)]
static mut el0_kernel_ttbr0: u64 = 0;
#[unsafe(no_mangle)]
static mut el0_run_sp: u64 = 0;
#[unsafe(no_mangle)]
static mut EL0_USER_TTBR: u64 = 0;

/// Saved user context for SVC resume (`TrapFrame` field order without pad).
#[repr(C)]
struct SavedUser {
    gpr: [u64; 31],
    elr: u64,
    spsr: u64,
}

#[unsafe(no_mangle)]
static mut EL0_SAVED: SavedUser = SavedUser {
    gpr: [0; 31],
    elr: 0,
    spsr: 0,
};

/// User `SP_EL0` at the SVC (kernel finish overwrites `SP_EL0` with its frame).
#[unsafe(no_mangle)]
static mut EL0_SAVED_SP_EL0: u64 = 0;

/// `SPSR_EL1` installed on the next [`enter`] (resume restores saved SPSR).
#[unsafe(no_mangle)]
static mut EL0_ENTRY_SPSR: u64 = SPSR_EL0_IRQS_MASKED;

unsafe extern "C" {
    fn el0_run(user_ttbr: u64, entry: u64, user_sp: u64, kernel_ttbr: u64) -> u64;
    fn el0_resume() -> u64;
}

#[unsafe(no_mangle)]
pub extern "C" fn exception_sync_el0(frame: &mut TrapFrame) {
    let esr = read_esr_el1();
    let far = read_far_el1();
    let ec = (esr >> 26) & 0x3F;
    let kind: u64 = match ec {
        0x15 => 1,
        0x24 => 2,
        _ => 3,
    };
    // SAFETY: called from `vectors.s` with the CPU already through
    // `kernel_entry` and the kernel root reinstalled, so this is the only
    // context running. It writes the single-slot session state that
    // `el0_run_finish` and `unpack` read immediately afterwards; `frame` is the
    // trap frame the vector just built on SP_EL1, valid for this call.
    unsafe {
        EL0_ESR = esr;
        EL0_FAR = far;
        EL0_KIND = kind;
        if kind == 1 {
            // AArch64 SVC: ELR is already the insn *after* the SVC — do not +4.
            EL0_SAVED.gpr = frame.gpr;
            EL0_SAVED.elr = frame.elr;
            EL0_SAVED.spsr = frame.spsr;
            // finish repurposes SP_EL0 as the kernel frame; keep the user SP.
            EL0_SAVED_SP_EL0 = read_sp_el0();
            EL0_CAN_RESUME = 1;
        } else {
            EL0_CAN_RESUME = 0;
        }
    }
}

/// IRQ from EL0: save user context for [`resume`] (ELR is the interrupted insn).
///
/// Vectors restore kernel `TTBR0` first. Caller should run the IRQ subsystem
/// then [`resume`]. Sessions that keep IRQs masked never reach here.
#[unsafe(no_mangle)]
pub extern "C" fn exception_irq_el0(frame: &mut TrapFrame) {
    // SAFETY: as `exception_sync_el0` — sole context, single-slot state, and a
    // trap frame the vector path just built. `ELR` is deliberately left at the
    // interrupted instruction: the architecture re-executes it on resume, so a
    // software skip here would silently drop one user instruction per IRQ.
    unsafe {
        EL0_KIND = 4;
        EL0_ESR = 0;
        EL0_FAR = 0;
        EL0_SAVED.gpr = frame.gpr;
        EL0_SAVED.elr = frame.elr;
        EL0_SAVED.spsr = frame.spsr;
        EL0_SAVED_SP_EL0 = read_sp_el0();
        EL0_CAN_RESUME = 1;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn el0_missing_kernel_ttbr() -> ! {
    panic!("lower-EL exception without published kernel TTBR0 (no live el0 session)");
}

core::arch::global_asm!(
    r#"
    .global el0_run
    .global el0_resume
    .global el0_run_finish
    .global el0_kernel_ttbr0
    .text

    // x0=user_ttbr, x1=entry, x2=user_sp, x3=kernel_ttbr
    el0_run:
        stp x29, x30, [sp, #-96]!
        mov x29, sp
        stp x19, x20, [sp, #16]
        stp x21, x22, [sp, #32]
        stp x23, x24, [sp, #48]
        stp x25, x26, [sp, #64]
        stp x27, x28, [sp, #80]

        adrp x9, el0_kernel_ttbr0
        add x9, x9, :lo12:el0_kernel_ttbr0
        str x3, [x9]

        mov x9, sp
        adrp x10, el0_run_sp
        add x10, x10, :lo12:el0_run_sp
        str x9, [x10]

        msr spsel, #1
        mov x19, x1
        mov x20, x2
        bl switch_ttbr0
        msr sp_el0, x20
        msr elr_el1, x19
        adrp x4, EL0_ENTRY_SPSR
        add x4, x4, :lo12:EL0_ENTRY_SPSR
        ldr x4, [x4]
        msr spsr_el1, x4

        mov x0, xzr
        mov x1, xzr
        mov x2, xzr
        mov x3, xzr
        mov x4, xzr
        eret

    // Resume after SVC/IRQ: user TTBR + EL0_SAVED → ERET.
    el0_resume:
        stp x29, x30, [sp, #-96]!
        mov x29, sp
        stp x19, x20, [sp, #16]
        stp x21, x22, [sp, #32]
        stp x23, x24, [sp, #48]
        stp x25, x26, [sp, #64]
        stp x27, x28, [sp, #80]

        mov x9, sp
        adrp x10, el0_run_sp
        add x10, x10, :lo12:el0_run_sp
        str x9, [x10]

        msr spsel, #1
        adrp x0, EL0_USER_TTBR
        add x0, x0, :lo12:EL0_USER_TTBR
        ldr x0, [x0]
        bl switch_ttbr0

        // Restore user SP_EL0 before ERET (finish clobbered it with kernel SP).
        adrp x0, EL0_SAVED_SP_EL0
        add x0, x0, :lo12:EL0_SAVED_SP_EL0
        ldr x0, [x0]
        msr sp_el0, x0

        // Base for EL0_SAVED in x29 (restored last). Offsets: gpr[i]=i*8, elr=0xF8, spsr=0x100.
        adrp x29, EL0_SAVED
        add x29, x29, :lo12:EL0_SAVED
        ldr x10, [x29, #0xF8]
        ldr x11, [x29, #0x100]
        msr elr_el1, x10
        msr spsr_el1, x11
        ldp x0,  x1,  [x29, #0x00]
        ldp x2,  x3,  [x29, #0x10]
        ldp x4,  x5,  [x29, #0x20]
        ldp x6,  x7,  [x29, #0x30]
        ldp x8,  x9,  [x29, #0x40]
        ldp x10, x11, [x29, #0x50]
        ldp x12, x13, [x29, #0x60]
        ldp x14, x15, [x29, #0x70]
        ldp x16, x17, [x29, #0x80]
        ldp x18, x19, [x29, #0x90]
        ldp x20, x21, [x29, #0xA0]
        ldp x22, x23, [x29, #0xB0]
        ldp x24, x25, [x29, #0xC0]
        ldp x26, x27, [x29, #0xD0]
        ldr x28,      [x29, #0xE0]
        ldr x30,      [x29, #0xF0]
        ldr x29,      [x29, #0xE8]
        eret

    // Vectors: pack outcome. Clear kernel TTBR only when not resumable (fault).
    el0_run_finish:
        adrp x9, EL0_CAN_RESUME
        add x9, x9, :lo12:EL0_CAN_RESUME
        ldr x10, [x9]
        cbnz x10, 1f
        adrp x9, el0_kernel_ttbr0
        add x9, x9, :lo12:el0_kernel_ttbr0
        str xzr, [x9]
1:
        adrp x9, el0_run_sp
        add x9, x9, :lo12:el0_run_sp
        ldr x9, [x9]
        msr spsel, #0
        mov sp, x9

        adrp x0, EL0_KIND
        add x0, x0, :lo12:EL0_KIND
        ldr x0, [x0]
        adrp x1, EL0_ESR
        add x1, x1, :lo12:EL0_ESR
        ldr x1, [x1]
        and x2, x1, #0xFFFF
        orr x0, x0, x2, lsl #32

        ldp x19, x20, [sp, #16]
        ldp x21, x22, [sp, #32]
        ldp x23, x24, [sp, #48]
        ldp x25, x26, [sp, #64]
        ldp x27, x28, [sp, #80]
        ldp x29, x30, [sp], #96
        ret
    "#
);

#[inline]
fn read_sp_el0() -> u64 {
    let v: u64;
    // SAFETY: `SP_EL0` is readable at EL1 as an ordinary system register. The
    // kernel runs *on* SP_EL0 (boot.s clears SPSel), so between an EL0 entry and
    // the vector's `msr spsel, #1` this reads the user stack pointer, which is
    // the only window the callers below use.
    unsafe {
        core::arch::asm!("mrs {}, sp_el0", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}
