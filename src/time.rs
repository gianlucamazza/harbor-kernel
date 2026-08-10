//! Kernel timekeeping — tick counter and timer IRQ policy.
//!
//! The counter is written by the timer IRQ and read by the main loop, so it is
//! atomic (architecture rule 7). The M1-era ban on atomics applied while the
//! MMU was off; since M2 the RAM is Normal WB Inner-Shareable and `LDXR`/`STXR`
//! behave. A plain `static mut` here would not merely be untidy: the main loop
//! reads `ticks()` in a spin that ends in `WFI`, and a non-atomic read is free
//! to be hoisted out of that loop.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::timer;
use crate::irq;

/// Monotonic tick counter. Producer: timer IRQ. Consumer: main loop.
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Periods that expired without their interrupt being serviced.
///
/// The deadline is absolute, so a late handler does not shift the series — but
/// the ticks it slept through still happened. They are added to the count, so
/// `ticks()` stays a measure of elapsed time rather than of arrivals, and they
/// are counted separately, because "this kernel is missing deadlines" is not
/// something a tick number can express.
static MISSED: AtomicU64 = AtomicU64::new(0);

/// IRQ handler registered for the platform timer line (BSP supplies the id).
///
/// Always re-arms **this** core's CNTP (ADR-0078/0079). Only affinity 0 advances
/// the global tick counter and signals timer waiters — dual producers would
/// double the rate. Must not perform console I/O. Cookie is the ADR-0008 id
/// (timer = 1).
pub fn on_timer_irq(cookie: u32) {
    let missed = timer::on_interrupt();
    // Secondary: local quantum IRQ only — no global timekeeping.
    if crate::arch::cpu::affinity() != 0 {
        return;
    }
    if missed != 0 {
        MISSED.fetch_add(missed, Ordering::Relaxed);
    }
    // One for this deadline, plus the ones nobody was there to serve: time
    // passed for all of them. Release: state the handler updated before this
    // is visible to a reader that observes the new count.
    TICKS.fetch_add(missed + 1, Ordering::Release);
    // ADR-0028: wake a task parked on this cookie (if any).
    irq::wait::signal(cookie);
}

/// Timer deadlines that expired unserviced since boot.
#[inline]
pub fn missed_ticks() -> u64 {
    MISSED.load(Ordering::Relaxed)
}

/// Monotonic tick count since the timer IRQ path became live.
#[inline]
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Acquire)
}
