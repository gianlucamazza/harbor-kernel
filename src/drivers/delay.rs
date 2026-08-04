//! Blocking delay contract and the arch-timer implementation.
//!
//! Shape aligned with embedded-hal 1.0 `DelayNs`. Multi-millisecond panel
//! sequencing uses this path; short pad settles may still use
//! [`crate::arch::mmio::spin_cycles`].

use crate::arch::timer;

/// Blocking wait measured in wall time.
pub trait DelayNs {
    /// Wait for at least `ns` nanoseconds.
    fn delay_ns(&mut self, ns: u32);

    /// Wait for at least `us` microseconds.
    #[inline]
    fn delay_us(&mut self, us: u32) {
        // 1000 ns per µs; saturating keeps a pathological `us` from wrapping.
        self.delay_ns(us.saturating_mul(1_000));
    }

    /// Wait for at least `ms` milliseconds.
    #[inline]
    fn delay_ms(&mut self, ms: u32) {
        self.delay_ns(ms.saturating_mul(1_000_000));
    }
}

/// Delay backed by the ARM Generic Timer physical counter.
///
/// Stateless: frequency is read from `CNTFRQ_EL0` on each wait so bring-up
/// before `timer::init` still works (panel init does not need the periodic
/// tick programmed).
#[derive(Clone, Copy, Debug, Default)]
pub struct ArchTimerDelay;

impl DelayNs for ArchTimerDelay {
    #[inline]
    fn delay_ns(&mut self, ns: u32) {
        timer::busy_wait_ns(u64::from(ns));
    }

    #[inline]
    fn delay_us(&mut self, us: u32) {
        // Prefer the dedicated path (clearer scaling) over ns conversion.
        timer::busy_wait_us(us);
    }

    #[inline]
    fn delay_ms(&mut self, ms: u32) {
        timer::busy_wait_ms(ms);
    }
}
