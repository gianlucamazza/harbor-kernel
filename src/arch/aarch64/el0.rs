//! EL0 entry/exit for M5 v1 (ADR-0014).
//!
//! ## Protocol (single owner — no split TTBR policy)
//!
//! 1. [`run`] publishes the kernel root in [`el0_kernel_ttbr0`], calls
//!    [`switch_ttbr0`](crate::arch::mmu::switch_ttbr0) for the user root,
//!    programs `ELR`/`SPSR`/`SP_EL0`, `ERET` to EL0.
//! 2. First lower-EL sync (`vectors.s`): **`kernel_entry` first** (exception
//!    stack is in the cloned kernel map under the still-live user `TTBR0`, and
//!    must run before any `bl` that would clobber EL0 GPRs), then
//!    `switch_ttbr0(kernel)`, classify ESR, drop the trap frame, return through
//!    [`el0_run_finish`].
//! 3. Caller resumes at EL1 with kernel `TTBR0` live (ADR-0014 preferred policy).
//!
//! M5 sessions run with IRQs masked. An IRQ from EL0 is therefore a contract
//! violation and panics after the same TTBR restore — not a silent `ERET`.
//! FIQ/SError from lower EL restore the session kernel root when published,
//! then take the unexpected path (never `switch_ttbr0(0)`).

use crate::arch::mmu;

/// Result of a one-shot EL0 run (first lower-EL sync only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum El0Outcome {
    /// `SVC` from AArch64 EL0 (ESR.EC = 0x15).
    Svc { imm: u16 },
    /// Data abort from lower EL (ESR.EC = 0x24).
    DataAbort { esr: u64, far: u64 },
    /// Other sync from lower EL (instruction abort, etc.).
    OtherSync { esr: u64, far: u64 },
}

/// Run `entry` at EL0 until the first lower-EL sync exception.
///
/// # Safety
/// - `ttbr0_phys` is a prepared user root (kernel coverage + user window).
/// - `entry` / `user_sp` are valid under that root.
/// - IRQs are masked for the whole call (M5 session contract).
/// - Only one session at a time (static session state).
pub unsafe fn run(ttbr0_phys: usize, entry: u64, user_sp: u64) -> El0Outcome {
    let Some(kernel_ttbr) = mmu::kernel_root_phys() else {
        // Without a kernel root the vector cannot restore ADR-0014 policy.
        panic!("el0::run: kernel map not activated");
    };
    // SAFETY: sole session; tables prepared; IRQs masked by caller.
    let packed = unsafe { el0_run(ttbr0_phys as u64, entry, user_sp, kernel_ttbr as u64) };
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

/// Written by [`exception_sync_el0`] before [`el0_run_finish`].
#[unsafe(no_mangle)]
static mut EL0_ESR: u64 = 0;
#[unsafe(no_mangle)]
static mut EL0_FAR: u64 = 0;
#[unsafe(no_mangle)]
static mut EL0_KIND: u64 = 0;

/// Kernel `TTBR0` for the active session. Vectors read this before any kernel C.
///
/// Zero means “no EL0 session” — lower-EL entry must not invent a root.
#[unsafe(no_mangle)]
static mut el0_kernel_ttbr0: u64 = 0;

/// `SP_EL0` of [`el0_run`]'s frame (kernel stack) for the return path.
#[unsafe(no_mangle)]
static mut el0_run_sp: u64 = 0;

unsafe extern "C" {
    /// Packed: low 32 = kind (1=svc, 2=data, 3=other), high 32 = SVC imm.
    fn el0_run(user_ttbr: u64, entry: u64, user_sp: u64, kernel_ttbr: u64) -> u64;
}

/// Classify the lower-EL sync; vectors then free the trap frame and finish.
#[unsafe(no_mangle)]
pub extern "C" fn exception_sync_el0(_frame: &mut crate::arch::exception::TrapFrame) {
    let esr = read_esr_el1();
    let far = read_far_el1();
    let ec = (esr >> 26) & 0x3F;
    // EC values for exceptions *from a lower EL* only (this vector group).
    let kind: u64 = match ec {
        0x15 => 1, // SVC, AArch64
        0x24 => 2, // Data abort, lower EL
        _ => 3,
    };
    unsafe {
        EL0_ESR = esr;
        EL0_FAR = far;
        EL0_KIND = kind;
    }
}

/// IRQ while an EL0 session is live. M5 masks IRQs around [`run`]; reaching
/// here means the mask was lost — fail closed after TTBR restore in the stub.
#[unsafe(no_mangle)]
pub extern "C" fn exception_irq_el0() -> ! {
    panic!("IRQ from EL0 during one-shot session (IRQs must stay masked)");
}

/// Lower-EL sync/IRQ path found `el0_kernel_ttbr0 == 0` (no published session).
///
/// Vectors refuse to call [`crate::arch::mmu::switch_ttbr0`] with a null root.
#[unsafe(no_mangle)]
pub extern "C" fn el0_missing_kernel_ttbr() -> ! {
    panic!("lower-EL exception without published kernel TTBR0 (no live el0 session)");
}

core::arch::global_asm!(
    r#"
    .global el0_run
    .global el0_run_finish
    .global el0_kernel_ttbr0
    .text

    // x0=user_ttbr, x1=entry, x2=user_sp, x3=kernel_ttbr
    //
    // Stack: kernel uses SPSel=0 (sp ≡ SP_EL0). Lower-EL entry forces SPSel=1
    // (sp ≡ SP_EL1). el0_run_sp records this frame's SP_EL0 for the ret path.
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

        // MSR SP_EL0 is UNDEFINED while SPSel=0 (SP_EL0 is the current SP).
        // Switch to SP_EL1 (exception stack) for the call and SP_EL0 write.
        msr spsel, #1
        mov x19, x1                 // entry
        mov x20, x2                 // user_sp
        // x0 = user_ttbr → sole TTBR switch (mmu::switch_ttbr0).
        bl switch_ttbr0
        msr sp_el0, x20
        msr elr_el1, x19
        mov x4, #0x3c0              // EL0t, DAIF masked
        msr spsr_el1, x4

        mov x0, xzr
        mov x1, xzr
        mov x2, xzr
        mov x3, xzr
        mov x4, xzr
        eret

    // Vectors: kernel TTBR restored, trap frame already dropped, SPSel=1.
    el0_run_finish:
        // End session: lower-EL stubs must not restore a stale root later.
        adrp x9, el0_kernel_ttbr0
        add x9, x9, :lo12:el0_kernel_ttbr0
        str xzr, [x9]

        adrp x9, el0_run_sp
        add x9, x9, :lo12:el0_run_sp
        ldr x9, [x9]
        // SPSel=1 here (handler). Select SP_EL0 *before* mov sp, else mov would
        // write SP_EL1 and msr spsel,#0 would expose the user SP_EL0.
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
