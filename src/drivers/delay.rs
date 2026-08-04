//! Blocking delay contract and the arch-timer implementation.
//!
//! Shape aligned with embedded-hal 1.0 `DelayNs` (nanosecond base). Convenience
//! microsecond waits are provided; millisecond helpers land with the first
//! caller that needs them (panel power-on sequences).

use crate::arch::timer;

/// Blocking wait measured in wall time.
pub trait DelayNs {
    /// Wait for at least `ns` nanoseconds.
    fn delay_ns(&mut self, ns: u32);

    /// Wait for at least `us` microseconds.
    #[inline]
    fn delay_us(&mut self, us: u32) {
        self.delay_ns(us.saturating_mul(1_000));
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
        timer::busy_wait_us(us);
    }
}
