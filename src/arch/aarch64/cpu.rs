//! CPU control helpers for AArch64.

/// Wait for event (low-power idle until SEV/interrupt).
#[inline(always)]
pub fn wait_for_event() {
    // SAFETY: `wfe` is a pure idle hint.
    unsafe {
        core::arch::asm!("wfe", options(nomem, nostack, preserves_flags));
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

/// Enable FP/Advanced SIMD at EL1 (and EL0 when used later).
///
/// Required once the compiler emits NEON/`fmov` (e.g. after MMU+caches are on).
/// Without this, ESR EC=0x07 traps on the first SIMD/FP instruction.
#[inline]
pub fn enable_fp_simd() {
    // SAFETY: CPACR_EL1 is EL1-accessible; FPEN=0b11 enables full FP/ASIMD.
    unsafe {
        core::arch::asm!(
            "mrs {tmp}, cpacr_el1",
            "orr {tmp}, {tmp}, {fpen}",
            "msr cpacr_el1, {tmp}",
            "isb",
            tmp = out(reg) _,
            fpen = const (0b11u64 << 20),
            options(nostack),
        );
    }
}
