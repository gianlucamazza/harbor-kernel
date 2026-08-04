//! Kernel timekeeping — tick counter and timer IRQ policy.
//!
//! M1 runs single-core with the MMU off. Hardware atomics (`LDXR`/`STXR`) are
//! unreliable without proper memory attributes; use plain counters until M2.

use crate::arch::timer;

// Single-core exclusive: only the IRQ path / bootstrap masked path write this.
static mut TICKS: u64 = 0;

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
    // SAFETY: single core; callers hold exclusive context (IRQ or bootstrap).
    unsafe {
        TICKS = TICKS.wrapping_add(1);
    }
}

/// Monotonic tick count since timer IRQ path became live.
#[inline]
pub fn ticks() -> u64 {
    // SAFETY: single core; atomicity not required.
    unsafe { TICKS }
}
