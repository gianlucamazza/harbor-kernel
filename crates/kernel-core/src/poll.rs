//! Bounded waiting on a hardware condition.
//!
//! Every `while status_bit_set() {}` in a driver is a hang waiting for a
//! wedged device, and the paths that most need to survive one — a panic
//! handler that has already masked interrupts — are the least able to report
//! that they are stuck. Giving the wait a budget turns "the board went quiet"
//! into "a character was dropped".
//!
//! Here rather than in the driver so the budget behaviour is testable: a
//! condition that never clears is exactly what cannot be produced on demand
//! from real hardware.

/// Poll `ready` until it returns `true`, at most `budget` times.
///
/// Returns `true` if the condition was observed, `false` if the budget ran out.
/// A `budget` of zero checks once: the caller asked for no *waiting*, not for
/// no *look*.
pub fn until(budget: u32, mut ready: impl FnMut() -> bool) -> bool {
    let mut attempts = 0;
    loop {
        if ready() {
            return true;
        }
        if attempts >= budget {
            return false;
        }
        attempts += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_condition_that_never_clears_gives_up() {
        assert!(!until(100, || false));
    }

    /// The point of the whole module: bounded means bounded. Without a budget
    /// this call would not return, and no test could say so.
    #[test]
    fn giving_up_takes_the_budget_and_not_a_step_more() {
        let mut looks = 0;
        assert!(!until(10, || {
            looks += 1;
            false
        }));
        // One look per attempt, plus the first one before any waiting.
        assert_eq!(looks, 11);
    }

    #[test]
    fn an_already_ready_condition_costs_one_look() {
        let mut looks = 0;
        assert!(until(1_000_000, || {
            looks += 1;
            true
        }));
        assert_eq!(looks, 1);
    }

    #[test]
    fn a_condition_that_clears_in_time_succeeds() {
        let mut looks = 0;
        assert!(until(10, || {
            looks += 1;
            looks == 5
        }));
        assert_eq!(looks, 5);
    }

    #[test]
    fn a_zero_budget_still_looks_once() {
        let mut looks = 0;
        assert!(!until(0, || {
            looks += 1;
            false
        }));
        assert_eq!(looks, 1);
    }
}
