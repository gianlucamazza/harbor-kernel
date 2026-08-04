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

/// Why the timer could not be programmed at the requested rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerError {
    /// A rate of zero has no interval.
    ZeroRate,
    /// `CNTFRQ_EL0` reads zero — the firmware did not program the counter.
    NoCounterFrequency,
    /// The requested rate is faster than the counter can express.
    RateTooHigh { requested_hz: u32, counter_hz: u64 },
}

/// Program a periodic physical timer at `hz` ticks per second and start it.
///
/// Does not touch the GIC. Caller must enable PPI 30 and unmask DAIF.I.
///
/// Returns an error rather than panicking: a board that cannot start its timer
/// can still run a polled console and report why, which is strictly more
/// useful than a kernel panic at boot.
pub fn init(hz: u32) -> Result<(), TimerError> {
    if hz == 0 {
        return Err(TimerError::ZeroRate);
    }
    let freq = frequency_hz();
    if freq == 0 {
        return Err(TimerError::NoCounterFrequency);
    }

    let interval = freq / u64::from(hz);
    if interval == 0 {
        return Err(TimerError::RateTooHigh {
            requested_hz: hz,
            counter_hz: freq,
        });
    }

    INTERVAL_COUNTS.store(interval, Ordering::Relaxed);
    write_tval(interval);
    // ENABLE=1, IMASK=0.
    write_ctl(0b001);
    Ok(())
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
