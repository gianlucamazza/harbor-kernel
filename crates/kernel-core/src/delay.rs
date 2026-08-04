//! Wall-time delay arithmetic for a free-running counter.
//!
//! Converts a duration into counter ticks given a frequency in Hz. The
//! architecture layer samples the counter; this module only does the pure
//! conversion so the host can test the rounding and overflow behaviour.

/// Counter ticks that cover at least `ns` nanoseconds at `freq_hz`.
///
/// Returns zero when either input is zero. For a positive duration the result
/// is at least one tick so a sub-tick request still advances the wait loop
/// once rather than becoming a no-op that races the counter.
///
/// Saturates at `u64::MAX` if the product would overflow — a wait that long is
/// not expressible on a 64-bit counter, and saturating is safer than wrapping
/// into a short delay.
pub const fn ns_to_counts(freq_hz: u64, ns: u64) -> u64 {
    if freq_hz == 0 || ns == 0 {
        return 0;
    }
    // counts = freq * ns / 1_000_000_000, rounded up so we never undershoot.
    let num = (freq_hz as u128).saturating_mul(ns as u128);
    let den = 1_000_000_000u128;
    let counts = num.div_ceil(den);
    if counts == 0 {
        1
    } else if counts > u64::MAX as u128 {
        u64::MAX
    } else {
        counts as u64
    }
}

/// Counter ticks that cover at least `ms` milliseconds at `freq_hz`.
pub const fn ms_to_counts(freq_hz: u64, ms: u32) -> u64 {
    // ms * 1_000_000 ns; do it in two steps to keep intermediates clear.
    let ns = (ms as u64).saturating_mul(1_000_000);
    ns_to_counts(freq_hz, ns)
}

/// Counter ticks that cover at least `us` microseconds at `freq_hz`.
pub const fn us_to_counts(freq_hz: u64, us: u32) -> u64 {
    let ns = (us as u64).saturating_mul(1_000);
    ns_to_counts(freq_hz, ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_second_at_54_mhz_is_the_frequency() {
        // Pi 4 silicon CNTFRQ is 54 MHz; one second is exactly that many ticks.
        assert_eq!(ns_to_counts(54_000_000, 1_000_000_000), 54_000_000);
        assert_eq!(ms_to_counts(54_000_000, 1000), 54_000_000);
    }

    #[test]
    fn one_ms_at_54_mhz() {
        assert_eq!(ms_to_counts(54_000_000, 1), 54_000);
    }

    #[test]
    fn sub_tick_positive_duration_is_at_least_one_count() {
        // 1 ns at 1 Hz is a fraction of a tick; still must wait once.
        assert_eq!(ns_to_counts(1, 1), 1);
    }

    #[test]
    fn zero_inputs_yield_zero() {
        assert_eq!(ns_to_counts(0, 1_000), 0);
        assert_eq!(ns_to_counts(54_000_000, 0), 0);
        assert_eq!(ms_to_counts(54_000_000, 0), 0);
    }

    #[test]
    fn rounds_up_partial_ticks() {
        // 1 ns at 2 Hz: 2 * 1 / 1e9 = 0 with truncate; ceil → 1.
        assert_eq!(ns_to_counts(2, 1), 1);
        // Half a second at 3 Hz: 3 * 5e8 / 1e9 = 1.5 → 2.
        assert_eq!(ns_to_counts(3, 500_000_000), 2);
    }

    #[test]
    fn huge_duration_saturates_instead_of_wrapping() {
        assert_eq!(ns_to_counts(u64::MAX, u64::MAX), u64::MAX);
    }

    #[test]
    fn us_scale() {
        assert_eq!(us_to_counts(1_000_000, 1), 1);
        assert_eq!(us_to_counts(54_000_000, 1), 54);
    }
}
