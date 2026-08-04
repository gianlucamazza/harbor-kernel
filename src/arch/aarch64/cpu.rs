//! CPU control helpers for AArch64.

/// Wait for event (low-power idle until SEV/interrupt).
#[inline(always)]
pub fn wait_for_event() {
    // SAFETY: `wfe` is a pure idle hint.
    unsafe {
        core::arch::asm!("wfe", options(nomem, nostack, preserves_flags));
    }
}

/// Wait for interrupt (idle until an IRQ/FIQ is taken or already pending).
///
/// Preferred for the main idle loop when work is purely IRQ-driven.
#[inline(always)]
pub fn wait_for_interrupt() {
    // SAFETY: `wfi` is a pure idle hint.
    unsafe {
        core::arch::asm!("wfi", options(nomem, nostack, preserves_flags));
    }
}

/// Park this core forever.
#[inline(always)]
pub fn halt() -> ! {
    loop {
        wait_for_event();
    }
}

/// Mask IRQs (`DAIF.I = 1`).
#[inline(always)]
pub fn irq_disable() {
    // SAFETY: DAIF update is a pure PSTATE change.
    unsafe {
        core::arch::asm!("msr daifset, #2", options(nomem, nostack, preserves_flags));
    }
}

/// Unmask IRQs (`DAIF.I = 0`).
#[inline(always)]
pub fn irq_enable() {
    // SAFETY: DAIF update is a pure PSTATE change.
    unsafe {
        core::arch::asm!("msr daifclr, #2", options(nomem, nostack, preserves_flags));
    }
}

/// Full system DSB then ISB — order device/config writes before unmasking IRQs.
#[inline(always)]
pub fn sync_pipeline() {
    // SAFETY: barrier instructions only.
    unsafe {
        core::arch::asm!("dsb sy", "isb", options(nostack, preserves_flags));
    }
}

// FP/Advanced SIMD is deliberately left trapping (`CPACR_EL1.FPEN` = 0).
//
// The kernel is built for `aarch64-unknown-none-softfloat`, so it emits no FP
// or SIMD instructions; `make no-simd` enforces that on the linked image. With
// FPEN clear, a stray FP instruction raises ESR EC=0x07 and is diagnosed,
// instead of running against an exception path (`vectors.s`) that saves no
// q registers and would corrupt the interrupted code's FP state.
//
// When EL0 agents need FP (M5), the shape to add here is lazy switching: trap
// on first use per task, save/restore q0–q31 + FPCR/FPSR only for tasks that
// actually touched the FPU. The kernel itself stays softfloat.
