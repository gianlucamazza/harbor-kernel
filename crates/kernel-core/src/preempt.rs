//! IRQ-side preemption decision (ADR-0064 / K4) — pure, host-tested.
//!
//! One question — "should the safe point rotate the current task?" — and the
//! answer is this module: the ADR-0046 quantum arithmetic plus the idle
//! guard. The predicate is monotone in `now`, which is why the kernel needs
//! no `need_resched` carrier between the tick and the safe point: the
//! agent-loop resume boundary evaluates [`should_set`] directly and performs
//! the switch. Nothing outside this module decides anything it does not.

use crate::budget;

/// True when the running slice has consumed its quantum and the current task
/// is not idle (idle is never preempted — there is nothing fairer than
/// nothing to run).
#[inline]
pub const fn should_set(slice_start: u64, now: u64, quantum: u64, current_is_idle: bool) -> bool {
    !current_is_idle && budget::expired(slice_start, now, quantum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_at_and_after_quantum() {
        assert!(should_set(10, 12, 2, false));
        assert!(should_set(10, 100, 2, false));
    }

    #[test]
    fn not_before_quantum() {
        assert!(!should_set(10, 10, 2, false));
        assert!(!should_set(10, 11, 2, false));
    }

    #[test]
    fn idle_never_sets() {
        assert!(!should_set(10, 100, 2, true));
    }

    #[test]
    fn zero_quantum_never_sets() {
        assert!(!should_set(0, u64::MAX, 0, false));
    }
}
