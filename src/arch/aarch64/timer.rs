//! ARM Generic Timer — EL1 physical timer (`CNTP_*`).
//!
//! Board-agnostic. Interrupt routing (PPI 30 → GIC) is the BSP's job.

use core::sync::atomic::{AtomicU64, Ordering};

use kernel_core::timer;

/// Interval between ticks, in timer counts (derived from `CNTFRQ_EL0`).
///
/// Written once by `init` during bootstrap, read from the IRQ path. Relaxed is
/// enough: the value is published before interrupts are ever unmasked.
static INTERVAL_COUNTS: AtomicU64 = AtomicU64::new(0);

/// The deadline currently programmed into the comparator.
///
/// The series is anchored here, not on the counter at handler time: that is
/// what keeps the phase from sliding by one interrupt latency per tick.
static DEADLINE: AtomicU64 = AtomicU64::new(0);

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
    // The series starts here, and every later deadline is derived from this one
    // rather than from the count at the time the handler runs.
    let first = physical_count().saturating_add(interval);
    DEADLINE.store(first, Ordering::Relaxed);
    write_cval(first);
    // ENABLE=1, IMASK=0.
    write_ctl(0b001);
    Ok(())
}

/// Bring the next absolute deadline forward to roughly `counts` after *now*.
///
/// Updates [`DEADLINE`] so the next [`on_interrupt`] keeps the absolute phase
/// series (not a one-shot detour). Used when an EL0 session must observe a
/// timer IRQ promptly without waiting a full idle period.
///
/// **Caller must hold the EL1 IRQ mask** if the intent is for lower-EL to see
/// the tick: with DAIF.I clear at EL1 the line is claimed by
/// `exception_irq_el1` before `el0::enter` runs.
pub fn accelerate_next_tick(counts: u64) {
    let d = physical_count().saturating_add(counts.max(1));
    DEADLINE.store(d, Ordering::Relaxed);
    write_cval(d);
    write_ctl(0b001);
}

/// Re-arm the next deadline. Called from the IRQ path only.
///
/// Returns the number of periods that expired unserviced, which is normally
/// zero. See [`kernel_core::timer`] for why the deadline is absolute.
pub fn on_interrupt() -> u64 {
    let interval = INTERVAL_COUNTS.load(Ordering::Relaxed);
    let previous = DEADLINE.load(Ordering::Relaxed);
    let next = timer::next_deadline(previous, interval, physical_count());

    DEADLINE.store(next.deadline, Ordering::Relaxed);
    write_cval(next.deadline);
    // Keep ENABLE=1, IMASK=0 after reprogram.
    write_ctl(0b001);
    next.missed
}

/// Current physical counter (`CNTPCT_EL0`).
#[inline]
pub fn physical_count() -> u64 {
    let count: u64;
    // SAFETY: CNTPCT_EL0 is readable at EL1. `isb` orders the read against
    // preceding instructions — without it the counter may be sampled early,
    // which would put the deadline behind where the caller thinks it is.
    unsafe {
        core::arch::asm!(
            "isb",
            "mrs {}, cntpct_el0",
            out(reg) count,
            options(nomem, nostack, preserves_flags),
        );
    }
    count
}

/// Spin until at least `ns` nanoseconds have elapsed on the physical counter.
///
/// Uses `CNTFRQ_EL0` so it is correct before [`init`] programs the periodic
/// tick. A zero duration returns immediately. Does not mask IRQs: long panel
/// waits must not suppress the timer for hundreds of milliseconds.
#[inline]
pub fn busy_wait_ns(ns: u64) {
    let freq = frequency_hz();
    let counts = kernel_core::delay::ns_to_counts(freq, ns);
    busy_wait_counts(counts);
}

/// Spin until at least `us` microseconds have elapsed.
#[inline]
pub fn busy_wait_us(us: u32) {
    // Route through `busy_wait_ns` so one counter path is always linked.
    busy_wait_ns(u64::from(us).saturating_mul(1_000));
}

#[inline]
fn busy_wait_counts(counts: u64) {
    if counts == 0 {
        return;
    }
    let start = physical_count();
    // Wrapping subtract so a counter that passes 2^64 still terminates; on a
    // 54 MHz clock that overflow is not a practical concern for panel waits.
    while physical_count().wrapping_sub(start) < counts {
        core::hint::spin_loop();
    }
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

/// Program the absolute compare value (`CNTP_CVAL_EL0`).
#[inline]
fn write_cval(deadline: u64) {
    // SAFETY: CNTP_CVAL_EL0 is accessible at EL1 for the physical timer.
    unsafe {
        core::arch::asm!(
            "msr cntp_cval_el0, {v}",
            v = in(reg) deadline,
            options(nostack, preserves_flags),
        );
    }
}

/// Program a countdown (`CNTP_TVAL_EL0`). Bring-up only: a relative re-arm
/// drifts, which is why the periodic path uses [`write_cval`].
#[cfg(feature = "bringup")]
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
