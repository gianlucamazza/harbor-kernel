//! ARM Generic Timer — EL1 physical timer (`CNTP_*`).
//!
//! Board-agnostic. Interrupt routing (PPI 30 → GIC) is the BSP's job.

use core::sync::atomic::{AtomicU64, Ordering};

/// Interval between ticks, in timer counts (derived from `CNTFRQ_EL0`).
///
/// Written once by `init` during bootstrap, read from the IRQ path. Relaxed is
/// enough: the value is published before interrupts are ever unmasked.
static INTERVAL_COUNTS: AtomicU64 = AtomicU64::new(0);

/// Read the timer frequency programmed by platform firmware (Hz).
#[inline]
pub fn frequency_hz() -> u64 {
    let freq: u64;
    // SAFETY: CNTFRQ_EL0 is readable at EL1.
    unsafe {
        core::arch::asm!(
            "mrs {}, cntfrq_el0",
            out(reg) freq,
            options(nomem, nostack, preserves_flags)
        );
    }
    freq
}

/// Program a periodic physical timer at `hz` ticks per second and start it.
///
/// Does not touch the GIC. Caller must enable PPI 30 and unmask DAIF.I.
///
/// # Panics
///
/// Panics if `hz == 0`, `CNTFRQ_EL0 == 0`, or the derived interval is zero.
pub fn init(hz: u32) {
    assert!(hz > 0, "timer hz must be non-zero");
    let freq = frequency_hz();
    assert!(freq > 0, "CNTFRQ_EL0 is zero");

    let interval = freq / u64::from(hz);
    assert!(interval > 0, "timer interval underflows at requested hz");

    INTERVAL_COUNTS.store(interval, Ordering::Relaxed);
    write_tval(interval);
    // ENABLE=1, IMASK=0.
    write_ctl(0b001);
}

/// Re-arm the next deadline. Called from the IRQ path only.
pub fn on_interrupt() {
    write_tval(INTERVAL_COUNTS.load(Ordering::Relaxed));
    // Keep ENABLE=1, IMASK=0 after reprogram.
    write_ctl(0b001);
}

/// True if the physical timer condition is asserted (`CNTP_CTL.ISTATUS`).
///
/// Polling the timer is a bring-up technique; the production path waits for
/// the interrupt.
#[cfg(feature = "bringup")]
#[inline]
pub fn is_pending() -> bool {
    (read_ctl() & (1 << 2)) != 0
}

/// Program a one-shot relative deadline of `counts` timer ticks (ENABLE, unmasked).
///
/// Used by bring-up gates; normal periodic mode uses [`init`] / [`on_interrupt`].
#[cfg(feature = "bringup")]
pub fn set_deadline_counts(counts: u64) {
    write_tval(counts.max(1));
    write_ctl(0b001);
}

#[cfg(feature = "bringup")]
#[inline]
fn read_ctl() -> u64 {
    let value: u64;
    // SAFETY: CNTP_CTL_EL0 is accessible at EL1 for the physical timer.
    unsafe {
        core::arch::asm!(
            "mrs {}, cntp_ctl_el0",
            out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

#[inline]
fn write_tval(counts: u64) {
    // SAFETY: CNTP_TVAL_EL0 is accessible at EL1 for the physical timer.
    unsafe {
        core::arch::asm!(
            "msr cntp_tval_el0, {v}",
            v = in(reg) counts,
            options(nostack, preserves_flags),
        );
    }
}

#[inline]
fn write_ctl(value: u64) {
    // SAFETY: CNTP_CTL_EL0 is accessible at EL1 for the physical timer.
    unsafe {
        core::arch::asm!(
            "msr cntp_ctl_el0, {v}",
            "isb",
            v = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}
