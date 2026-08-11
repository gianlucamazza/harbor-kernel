//! Supervisor lifecycle verdicts (ADR-0092) — pure, host-tested.
//!
//! Three kernel entry points ask the same question and used to answer it in
//! three different orders written three frames apart: given who the target is
//! and what state it is in, what may a supervisor do to it?
//!
//! - [`cancel`] — ADR-0025, the wait-cancel path.
//! - [`reap`] — ADR-0033, the supervisor reaping a blocked child.
//! - [`force`] — ADR-0090, force-exit at the next safe point.
//!
//! Each takes the two facts the kernel can supply (whether the target is idle,
//! and its model state) and returns *what to do*, never *how*. Taking the
//! lock, writing the TCB flag, waking, counting, nudging the home CPU and
//! calling into `ipc` all stay on the kernel side.
//!
//! # The `Empty` divergence is deliberate
//!
//! [`reap`] answers `NotBlocked` for an empty slot and [`force`] answers
//! `Empty` for the same input. Both are right for their own question — reap is
//! about blockedness, force is about the slot — and both are ABI (ADR-0061
//! detail codes an EL0 agent reads). The tests below assert the two together
//! so the difference is a decision on record rather than an accident of
//! reading order.

use crate::tasks::State;

/// Why a supervisor reap was refused (ADR-0033).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReapError {
    /// Target is the idle task.
    Idle,
    /// Unknown task id (or stale epoch, ADR-0062).
    BadId,
    /// Task is not currently [`State::Blocked`] — an empty slot included.
    NotBlocked,
}

/// Why a force-exit was refused (ADR-0090).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForceError {
    /// Target is the idle task.
    Idle,
    /// Unknown task id (or stale epoch, ADR-0062).
    BadId,
    /// Slot already empty.
    Empty,
}

/// What [`reap`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReapVerdict {
    /// Cancel the wait. If the cancel does not land, the kernel reports
    /// [`ReapError::NotBlocked`] — the state was read before the lock the
    /// cancel takes, so losing the race is the same answer as never having
    /// been blocked.
    Cancel,
    /// Refuse, with the class the ABI reports.
    Refuse(ReapError),
}

/// What [`force`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForceVerdict {
    /// Set `force_exit` and cancel the wait: the victim is parked, so it needs
    /// waking before it can observe the flag.
    FlagAndCancel,
    /// Set `force_exit` and nudge the home CPU: the victim is runnable or
    /// running, so it reaches a safe point on its own once rescheduled.
    FlagAndNudge,
    /// Refuse, with the class the ABI reports.
    Refuse(ForceError),
}

/// What [`cancel`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelVerdict {
    /// Mark `cancel_wait` and wake the task.
    MarkAndWake,
    /// Do nothing. This path has no refusal classes — its one caller reports a
    /// bare `false` (ADR-0025).
    Refuse,
}

/// Reap verdict for a target that is `is_idle`, in `state` (ADR-0033).
///
/// `state` is `None` for an unknown id or a stale epoch (ADR-0062).
pub const fn reap(is_idle: bool, state: Option<State>) -> ReapVerdict {
    if is_idle {
        return ReapVerdict::Refuse(ReapError::Idle);
    }
    match state {
        None => ReapVerdict::Refuse(ReapError::BadId),
        Some(State::Blocked) => ReapVerdict::Cancel,
        // Empty lands here on purpose: reap asks about blockedness, and an
        // empty slot is not blocked. See the module doc.
        Some(_) => ReapVerdict::Refuse(ReapError::NotBlocked),
    }
}

/// Force-exit verdict for a target that is `is_idle`, in `state` (ADR-0090).
pub const fn force(is_idle: bool, state: Option<State>) -> ForceVerdict {
    if is_idle {
        return ForceVerdict::Refuse(ForceError::Idle);
    }
    match state {
        None => ForceVerdict::Refuse(ForceError::BadId),
        // Unlike `reap`: force asks about the slot, and "nothing here" is a
        // different fact from "not waiting". See the module doc.
        Some(State::Empty) => ForceVerdict::Refuse(ForceError::Empty),
        Some(State::Blocked) => ForceVerdict::FlagAndCancel,
        Some(_) => ForceVerdict::FlagAndNudge,
    }
}

/// Cancel-wait verdict for a target in `state` (ADR-0025).
///
/// Idle needs no separate arm: idle never blocks (model invariant), so it can
/// only reach the `Refuse` below.
pub const fn cancel(state: Option<State>) -> CancelVerdict {
    match state {
        Some(State::Blocked) => CancelVerdict::MarkAndWake,
        _ => CancelVerdict::Refuse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [Option<State>; 5] = [
        None,
        Some(State::Empty),
        Some(State::Ready),
        Some(State::Running),
        Some(State::Blocked),
    ];

    #[test]
    fn idle_is_refused_first_whatever_the_state() {
        for s in STATES {
            assert_eq!(reap(true, s), ReapVerdict::Refuse(ReapError::Idle));
            assert_eq!(force(true, s), ForceVerdict::Refuse(ForceError::Idle));
        }
    }

    #[test]
    fn an_unknown_or_stale_id_is_bad_id() {
        assert_eq!(reap(false, None), ReapVerdict::Refuse(ReapError::BadId));
        assert_eq!(force(false, None), ForceVerdict::Refuse(ForceError::BadId));
    }

    #[test]
    fn reap_cancels_only_a_blocked_task() {
        assert_eq!(reap(false, Some(State::Blocked)), ReapVerdict::Cancel);
        for s in [State::Empty, State::Ready, State::Running] {
            assert_eq!(
                reap(false, Some(s)),
                ReapVerdict::Refuse(ReapError::NotBlocked)
            );
        }
    }

    #[test]
    fn force_cancels_a_blocked_task_and_nudges_a_runnable_one() {
        assert_eq!(
            force(false, Some(State::Blocked)),
            ForceVerdict::FlagAndCancel
        );
        for s in [State::Ready, State::Running] {
            assert_eq!(force(false, Some(s)), ForceVerdict::FlagAndNudge);
        }
    }

    #[test]
    fn an_empty_slot_answers_differently_to_reap_and_to_force() {
        // Deliberate, and ABI (ADR-0061 detail codes): reap asks whether the
        // target is blocked — an empty slot is not — while force asks about
        // the slot itself, where "nothing here" is its own answer. If one of
        // these ever changes, it is an ABI change and this test is where the
        // decision has to be re-made.
        assert_eq!(
            reap(false, Some(State::Empty)),
            ReapVerdict::Refuse(ReapError::NotBlocked)
        );
        assert_eq!(
            force(false, Some(State::Empty)),
            ForceVerdict::Refuse(ForceError::Empty)
        );
    }

    #[test]
    fn cancel_marks_only_a_blocked_task() {
        assert_eq!(cancel(Some(State::Blocked)), CancelVerdict::MarkAndWake);
        for s in STATES {
            if s == Some(State::Blocked) {
                continue;
            }
            assert_eq!(cancel(s), CancelVerdict::Refuse);
        }
    }

    #[test]
    fn every_state_gets_exactly_one_verdict() {
        // A table with a hole would compile: `Option<State>` has five
        // inhabitants and each function matches on it. This walks all ten
        // (is_idle × state) inputs of each and asserts none panics or falls
        // into an unintended arm — the property a mutant that reorders the
        // arms would break.
        for idle in [true, false] {
            for s in STATES {
                let r = reap(idle, s);
                let f = force(idle, s);
                if idle {
                    assert_eq!(r, ReapVerdict::Refuse(ReapError::Idle));
                    assert_eq!(f, ForceVerdict::Refuse(ForceError::Idle));
                } else {
                    assert_eq!(r == ReapVerdict::Cancel, s == Some(State::Blocked));
                    assert_eq!(f == ForceVerdict::FlagAndCancel, s == Some(State::Blocked));
                }
            }
        }
    }

    #[test]
    fn a_blocked_task_is_the_only_input_all_three_agree_on() {
        // The one row where the three verdicts line up — worth pinning,
        // because it is the row every supervisor demo exercises.
        assert_eq!(reap(false, Some(State::Blocked)), ReapVerdict::Cancel);
        assert_eq!(
            force(false, Some(State::Blocked)),
            ForceVerdict::FlagAndCancel
        );
        assert_eq!(cancel(Some(State::Blocked)), CancelVerdict::MarkAndWake);
    }
}
