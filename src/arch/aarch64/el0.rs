//! EL0 entry / SVC resume (ADR-0014).
//!
//! ## Protocol
//!
//! 1. [`enter`] publishes the kernel root, switches to the user `TTBR0`,
//!    programs `ELR`/`SPSR`/`SP_EL0`, `ERET` to EL0.
//! 2. Lower-EL sync: `kernel_entry` → `switch_ttbr0(kernel)` → classify. On
//!    **SVC**, user GPRs/`ELR`/`SPSR` are saved and `ELR` advanced by 4 so
//!    [`resume`] can continue the same user context.
//! 3. [`resume`] re-installs the user root and `ERET`s with the saved state.
//! 4. [`end_session`] clears the published kernel root.
//!
//! [`run`] is one-shot: `enter` + [`end_session`]. IRQs stay masked for the
//! whole session.

use crate::arch::exception::TrapFrame;
use crate::arch::mmu;

/// Result of one EL0 stretch (enter or resume until the next lower-EL sync).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum El0Outcome {
    /// `SVC` from AArch64 EL0. Session may [`resume`].
    Svc { imm: u16 },
    /// Data abort from lower EL. Session ends.
    DataAbort { esr: u64, far: u64 },
    /// Other sync from lower EL. Session ends.
    OtherSync { esr: u64, far: u64 },
}

/// One-shot: enter EL0 until the first sync, then end the session.
///
/// # Safety
/// Same as [`enter`].
pub unsafe fn run(ttbr0_phys: usize, entry: u64, user_sp: u64) -> El0Outcome {
    let outcome = unsafe { enter(ttbr0_phys, entry, user_sp) };
    end_session();
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

/// Continue after [`El0Outcome::Svc`] (`ELR` already points past the SVC).
///
/// # Safety
/// Prior event was `Svc`; IRQs masked; session not ended.
pub unsafe fn resume() -> El0Outcome {
    if unsafe { EL0_CAN_RESUME } == 0 {
        panic!("el0::resume: no resumable SVC session");
    }
    if unsafe { el0_kernel_ttbr0 } == 0 {
        panic!("el0::resume: session kernel TTBR cleared");
    }
    unsafe { unpack(el0_resume()) }
}

/// Clear session symbols (call after the last event if not already cleared).
#[inline]
pub fn end_session() {
    unsafe {
        el0_kernel_ttbr0 = 0;
        EL0_CAN_RESUME = 0;
        EL0_USER_TTBR = 0;
    }
}

fn unpack(packed: u64) -> El0Outcome {
    let kind = (packed & 0xFFFF_FFFF) as u32;
    match kind {
        1 => El0Outcome::Svc {
            imm: (packed >> 32) as u16,
        },
        2 => El0Outcome::DataAbort {
            esr: unsafe { EL0_ESR },
            far: unsafe { EL0_FAR },
        },
        _ => El0Outcome::OtherSync {
            esr: unsafe { EL0_ESR },
            far: unsafe { EL0_FAR },
        },
    }
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

#[unsafe(no_mangle)]
pub extern "C" fn exception_irq_el0() -> ! {
    panic!("IRQ from EL0 during session (IRQs must stay masked)");
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
        mov x4, #0x3c0
        msr spsr_el1, x4

        mov x0, xzr
        mov x1, xzr
        mov x2, xzr
        mov x3, xzr
        mov x4, xzr
        eret

    // Resume after SVC: user TTBR + EL0_SAVED → ERET.
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
fn read_esr_el1() -> u64 {
    let v: u64;
    unsafe {
        core::arch::asm!("mrs {}, esr_el1", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}

#[inline]
fn read_far_el1() -> u64 {
    let v: u64;
    unsafe {
        core::arch::asm!("mrs {}, far_el1", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}

#[inline]
fn read_sp_el0() -> u64 {
    let v: u64;
    unsafe {
        core::arch::asm!("mrs {}, sp_el0", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}
