//! CPU control helpers for AArch64.

/// Wait for event (low-power idle until SEV/interrupt).
///
/// Deliberately **not** `nomem`: the caller waits for state another context
/// makes visible in memory, so this must act as a compiler barrier. Marking it
/// `nomem` would let the loop condition be hoisted out and never re-read.
#[inline(always)]
pub fn wait_for_event() {
    // SAFETY: `wfe` is an idle hint with no architectural side effects.
    unsafe {
        core::arch::asm!("wfe", options(nostack, preserves_flags));
    }
}

/// Wait for interrupt (idle until an IRQ/FIQ is taken or already pending).
///
/// Preferred for the main idle loop when work is purely IRQ-driven. See
/// [`wait_for_event`] for why this is not `nomem`.
///
/// `WFI` completes on a pending interrupt **even with `DAIF.I` set**, which is
/// what makes [`without_irqs`] the correct wrapper for check-then-sleep: an
/// interrupt arriving after the check cannot be lost.
#[inline(always)]
pub fn wait_for_interrupt() {
    // SAFETY: `wfi` is an idle hint with no architectural side effects.
    unsafe {
        core::arch::asm!("wfi", options(nostack, preserves_flags));
    }
}

/// Mask IRQs and return the previous `DAIF`, for a later [`irq_restore`].
///
/// The scheduler needs the open and the close of a critical section to be two
/// separate calls, because a context switch does not return on the stack it was
/// called from: a closure API cannot express that. Everything else should use
/// [`without_irqs`], which is written on top of this pair so the mask sequence
/// has exactly one definition.
#[inline]
#[must_use = "the saved DAIF must be handed to irq_restore or the section never closes"]
pub fn irq_save() -> u64 {
    let daif: u64;
    // SAFETY: reading DAIF and masking IRQs are pure PSTATE operations. Not
    // `nomem`: this opens a critical section around memory the IRQ path shares.
    unsafe {
        core::arch::asm!(
            "mrs {daif}, daif",
            "msr daifset, #2",
            daif = out(reg) daif,
            options(nostack, preserves_flags),
        );
    }
    daif
}

/// Restore a `DAIF` captured by [`irq_save`].
///
/// Restoring rather than unconditionally unmasking is the whole point: nested
/// use, or a call from a context that already had IRQs masked (bootstrap, an
/// exception handler), must not silently enable them on the way out.
///
/// # Safety
/// `daif` must come from an [`irq_save`] on this core whose section is being
/// closed here. Restoring a value from a different section re-opens or re-masks
/// interrupts against the intent of the code in between.
#[inline]
pub unsafe fn irq_restore(daif: u64) {
    // SAFETY: restores the exact PSTATE mask bits the caller captured.
    unsafe {
        core::arch::asm!(
            "msr daif, {daif}",
            daif = in(reg) daif,
            options(nostack, preserves_flags),
        );
    }
}

/// Run `f` with IRQs masked, then restore the previous `DAIF`.
#[inline]
pub fn without_irqs<R>(f: impl FnOnce() -> R) -> R {
    let daif = irq_save();
    let result = f();
    // SAFETY: closes the section opened immediately above.
    unsafe { irq_restore(daif) };
    result
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

/// Affinity level 0 of `MPIDR_EL1` — the core id within the cluster.
///
/// Used by K8 (ADR-0074) so IRQ-epilogue preemption stays on the primary
/// until per-core runqueues exist. One `mrs`, no decode tables.
#[inline]
pub fn affinity() -> u8 {
    let mpidr: u64;
    // SAFETY: reading MPIDR_EL1 has no side effects.
    unsafe {
        core::arch::asm!(
            "mrs {}, mpidr_el1",
            out(reg) mpidr,
            options(nomem, nostack, preserves_flags)
        );
    }
    (mpidr & 0xFF) as u8
}

/// `MIDR_EL1` — implementer, part and stepping of this core.
///
/// One `mrs`, no logic: the decode is [`kernel_core::cpuid`]'s, where it is
/// host-tested (ADR-0065) — the same split `PM_RSTS` has with
/// [`kernel_core::reset`].
#[inline]
pub fn midr_el1() -> u64 {
    let v: u64;
    // SAFETY: reading an ID register has no side effects.
    unsafe {
        core::arch::asm!("mrs {}, midr_el1", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}

/// `ID_AA64MMFR0_EL1` — memory-model features (ASID width, PA range, granules).
#[inline]
pub fn id_aa64mmfr0_el1() -> u64 {
    let v: u64;
    // SAFETY: reading an ID register has no side effects.
    unsafe {
        core::arch::asm!(
            "mrs {}, id_aa64mmfr0_el1",
            out(reg) v,
            options(nomem, nostack, preserves_flags)
        );
    }
    v
}

/// `ID_AA64PFR0_EL1` — processor features (EL support, FP/AdvSIMD).
#[inline]
pub fn id_aa64pfr0_el1() -> u64 {
    let v: u64;
    // SAFETY: reading an ID register has no side effects.
    unsafe {
        core::arch::asm!(
            "mrs {}, id_aa64pfr0_el1",
            out(reg) v,
            options(nomem, nostack, preserves_flags)
        );
    }
    v
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
