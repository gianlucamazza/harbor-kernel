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

/// Monotonic tick counter. Producer: timer IRQ. Consumer: main loop.
static TICKS: AtomicU64 = AtomicU64::new(0);

/// IRQ handler registered for the platform timer line (BSP supplies the id).
///
/// Re-arms the arch timer and advances the monotonic tick counter.
/// Must not perform console I/O.
pub fn on_timer_irq() {
    timer::on_interrupt();
    tick();
}

/// Advance the tick counter only (no hardware access).
#[inline]
pub fn tick() {
    // Release: any state the handler updated before this is visible to a
    // reader that observes the new count.
    TICKS.fetch_add(1, Ordering::Release);
}

/// Monotonic tick count since the timer IRQ path became live.
#[inline]
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Acquire)
}
