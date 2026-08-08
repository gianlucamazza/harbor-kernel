//! Cooperative CPU budget arithmetic (ADR-0046 / K4) — pure, host-tested.

/// True when the running slice has consumed at least `quantum` ticks.
#[inline]
pub const fn expired(slice_start: u64, now: u64, quantum: u64) -> bool {
    if quantum == 0 {
        return false;
    }
    now.saturating_sub(slice_start) >= quantum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_expired_before_quantum() {
        assert!(!expired(10, 11, 2));
        assert!(!expired(10, 10, 2));
    }

    #[test]
    fn expired_at_and_after_quantum() {
        assert!(expired(10, 12, 2));
        assert!(expired(10, 100, 2));
    }

    #[test]
    fn zero_quantum_never_expires() {
        assert!(!expired(0, u64::MAX, 0));
    }

    #[test]
    fn saturating_sub_handles_clock_skew() {
        // now < start should not panic or wrap into a huge elapsed.
        assert!(!expired(100, 50, 1));
    }
}
