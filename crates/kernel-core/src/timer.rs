//! Periodic deadline arithmetic for a one-shot comparator.
//!
//! The ARM Generic Timer offers two ways to re-arm: `CNTP_TVAL_EL0`, a
//! countdown from *now*, and `CNTP_CVAL_EL0`, an absolute compare value.
//!
//! Re-arming with `TVAL` from the handler looks periodic and is not. Every tick
//! starts counting when the handler runs, not when the deadline expired, so
//! each period absorbs the interrupt latency plus whatever a masked critical
//! section delayed. The error never cancels; it accumulates. At 10 Hz nobody
//! notices. As the time base of a scheduler it is a clock that runs slow by an
//! amount nothing measures.
//!
//! With an absolute deadline the phase is fixed by construction: the next
//! deadline is derived from the previous one, not from the current count, so
//! latency shifts a single tick and never the series.
//!
//! Which leaves the case that makes this more than an addition: what to do when
//! the new deadline is already in the past.

/// The next deadline, and what was lost getting to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextDeadline {
    /// Absolute counter value to program into the comparator.
    pub deadline: u64,
    /// Periods that elapsed unserviced before this one.
    ///
    /// Non-zero means the handler did not run in time — interrupts masked too
    /// long, or a period too short for the work. The tick count stays truthful
    /// because these are added to it; the number is also worth reporting, since
    /// a scheduler built on a clock that skips is a scheduler that stalls
    /// without saying so.
    pub missed: u64,
}

/// Advance a periodic deadline past `now`.
///
/// `previous` is the deadline that just expired, not the current count: that is
/// the whole point — phase comes from the series, not from when the handler
/// happened to run.
///
/// Always returns a deadline strictly greater than `now`. Re-arming into the
/// past would fire again immediately, and a handler that re-arms into the past
/// every time is an interrupt storm that looks like a hang.
///
/// `interval` of zero is treated as one count: a zero period would loop here
/// forever, and this is arithmetic on a value that came from a register.
pub const fn next_deadline(previous: u64, interval: u64, now: u64) -> NextDeadline {
    let interval = if interval == 0 { 1 } else { interval };

    // The ordinary case, and the one that must not drift: one interval on from
    // the deadline that expired.
    let Some(candidate) = previous.checked_add(interval) else {
        // 64 bits of a 54 MHz counter is over ten thousand years, so this is
        // unreachable in practice — but saturating beats wrapping into the past.
        return NextDeadline {
            deadline: u64::MAX,
            missed: 0,
        };
    };

    if candidate > now {
        return NextDeadline {
            deadline: candidate,
            missed: 0,
        };
    }

    // Behind. Skip whole periods rather than catching up one interrupt at a
    // time: replaying missed ticks would spend the handler's time reproducing
    // a backlog that is already late, and the phase is what we are preserving,
    // not the count of arrivals.
    let behind = now - previous;
    let periods = behind / interval + 1;
    match previous.checked_add(periods.saturating_mul(interval)) {
        Some(deadline) => NextDeadline {
            deadline,
            missed: periods - 1,
        },
        None => NextDeadline {
            deadline: u64::MAX,
            missed: periods - 1,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERVAL: u64 = 100;

    #[test]
    fn the_ordinary_case_is_one_interval_on_from_the_previous_deadline() {
        let next = next_deadline(1_000, INTERVAL, 1_005);
        assert_eq!(
            next.deadline, 1_100,
            "derived from the deadline, not from now"
        );
        assert_eq!(next.missed, 0);
    }

    /// The defect this exists to prevent. Re-arming relative to `now` would put
    /// each deadline at `now + interval`, so every tick keeps the latency it
    /// was served with and the series slides. Deriving from the previous
    /// deadline keeps the phase whatever the latency was.
    #[test]
    fn latency_does_not_accumulate_across_ticks() {
        let mut deadline = 1_000;
        for tick in 1..=100 {
            // Seven counts of handler latency, every time.
            let now = deadline + 7;
            deadline = next_deadline(deadline, INTERVAL, now).deadline;
            assert_eq!(
                deadline,
                1_000 + tick * INTERVAL,
                "tick {tick} drifted: the phase must come from the series"
            );
        }
    }

    #[test]
    fn a_deadline_already_past_skips_whole_periods() {
        // Served two and a half periods late.
        let next = next_deadline(1_000, INTERVAL, 1_250);
        assert!(next.deadline > 1_250, "must not re-arm into the past");
        assert_eq!(next.deadline, 1_300, "still on the original phase");
        assert_eq!(next.missed, 2);
    }

    /// A deadline landing exactly on `now` has already expired: programming it
    /// would fire immediately, which is the interrupt storm this guards.
    #[test]
    fn a_deadline_exactly_at_now_is_advanced() {
        let next = next_deadline(1_000, INTERVAL, 1_100);
        assert_eq!(next.deadline, 1_200);
        assert_eq!(next.missed, 1);
    }

    /// However far behind, one call is enough — the handler must not need N
    /// interrupts to work off N missed periods.
    #[test]
    fn one_call_catches_up_any_backlog() {
        let next = next_deadline(0, INTERVAL, 10_000_000);
        assert!(next.deadline > 10_000_000);
        assert_eq!(
            next.deadline % INTERVAL,
            0,
            "phase preserved after catch-up"
        );
        assert_eq!(next.missed, 100_000);
    }

    #[test]
    fn a_zero_interval_does_not_hang_or_divide_by_zero() {
        let next = next_deadline(10, 0, 10);
        assert!(next.deadline > 10);
    }

    #[test]
    fn a_deadline_near_the_end_of_the_counter_saturates_instead_of_wrapping() {
        let next = next_deadline(u64::MAX - 10, INTERVAL, u64::MAX - 5);
        assert_eq!(next.deadline, u64::MAX, "wrapping would arm in the past");
    }
}
