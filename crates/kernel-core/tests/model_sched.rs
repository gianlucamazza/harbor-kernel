//! Bounded exhaustive model check of the scheduler's state machine.
//!
//! Every other test in this crate picks a scenario someone thought of. This one
//! picks none: it replays **every** sequence of scheduler operations up to a
//! fixed depth, from a fresh [`Tasks`], and asserts the invariants after every
//! step. What it buys is stated precisely below, and what it does not buy is
//! stated with it — a bounded result sold as a general one would be worse than
//! no result.
//!
//! # Why this exists
//!
//! `Tasks::switch` carries a comment saying three mutants survive there and
//! *"no test can honestly cover it"*. The invariant they guard is **idle is
//! always exactly one of _current_ or _queued_**, and a hand-written test
//! cannot reach the branch behind it without first breaking the invariant —
//! which would be testing the test. That is a true statement about *chosen*
//! scenarios and a false one about *all* of them: an exhaustive walk either
//! holds the invariant over every reachable state within the bound, or prints
//! the sequence that breaks it.
//!
//! # What is bounded
//!
//! `Tasks<3>` — idle plus two workers — and sequences of at most [`DEPTH`]
//! operations. Both are small on purpose: the state machine's invariants do not
//! depend on the number of slots, so a violation that needs twelve tasks to
//! appear would be a violation of a different rule than the ones asserted here.
//! That is an **argument**, not a theorem, and it is the honest limit of this
//! file.
//!
//! No deduplication of visited states: sequences are replayed from scratch, so
//! the search cannot prune a path that a coarser state fingerprint would have
//! merged. It costs replay time and buys soundness within the bound.

use kernel_core::runqueue::TaskId;
use kernel_core::tasks::{Decision, State, Switch, Tasks};

/// Idle + two workers: the smallest table where a worker can exit while another
/// is ready, which is the shape every interesting invariant needs.
const SLOTS: usize = 3;

/// Longest operation sequence replayed. `ALPHABET^DEPTH` sequences, each
/// replayed from a fresh table, so the cost is `ALPHABET^DEPTH * DEPTH` steps.
const DEPTH: usize = 7;

/// One scheduler operation. Deliberately parameterless where the kernel's own
/// callers are: `switch` always acts on whoever is current, and `wake` is the
/// only action that names a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Admit,
    Collect,
    /// Wake the **live** id of this slot (no-op when the slot is empty) —
    /// which is how every kernel caller names a task since ADR-0062.
    Wake(usize),
    /// Wake a forged id for this slot whose epoch is one behind the current
    /// one. Always names a task that no longer exists; the model asserts the
    /// wake is refused and the machine is undisturbed (ADR-0062). One slot
    /// suffices: slots are symmetric.
    WakeStale(usize),
    Switch(Switch),
}

/// The alphabet, in a fixed order so a printed counter-example is reproducible.
const ALPHABET: [Op; 7 + SLOTS] = [
    Op::Admit,
    Op::Collect,
    Op::Switch(Switch::Yield),
    Op::Switch(Switch::Preempt),
    Op::Switch(Switch::Block),
    Op::Switch(Switch::Exit),
    Op::Wake(0),
    Op::Wake(1),
    Op::Wake(2),
    Op::WakeStale(1),
];

/// Forge the id a holder from the slot's previous tenancy would still hold.
fn stale_id_for(tasks: &Tasks<SLOTS>, slot: usize) -> TaskId {
    let epoch = tasks.live_id(slot).map(|id| id.epoch()).unwrap_or_default();
    TaskId::new(slot as u16, epoch.wrapping_sub(1))
}

/// Returns the decision (for `Switch`) and the first violation the operation
/// itself exposed (stale wake accepted), if any.
fn apply(tasks: &mut Tasks<SLOTS>, op: Op) -> (Option<Decision>, Option<&'static str>) {
    match op {
        Op::Admit => {
            tasks.admit();
            (None, None)
        }
        Op::Collect => {
            tasks.collect();
            (None, None)
        }
        Op::Wake(slot) => {
            if let Some(id) = tasks.live_id(slot) {
                tasks.wake(id);
            }
            (None, None)
        }
        Op::WakeStale(slot) => {
            let woke = tasks.wake(stale_id_for(tasks, slot));
            (None, woke.then_some("a stale id woke a task (ADR-0062)"))
        }
        Op::Switch(kind) => (Some(tasks.switch(kind)), None),
    }
}

/// Every invariant, checked after every step. Returns the first violation.
///
/// Expressed through the public API only — `current`, `state`, and the
/// `Decision` just returned. An invariant that needed private state would be an
/// invariant the kernel's callers cannot rely on.
fn violation(tasks: &Tasks<SLOTS>, decision: Option<Decision>) -> Option<&'static str> {
    let idle = Tasks::<SLOTS>::IDLE;

    // 1. The one the surviving mutants guard. Idle is popped when it runs and
    //    requeued when it yields, and it may neither block nor exit — so it is
    //    always exactly one of running or ready. Everything in `switch` that
    //    falls back to idle depends on this.
    let idle_state = tasks.state(idle);
    if tasks.current() != idle && idle_state != Some(State::Ready) {
        return Some("idle is neither current nor ready");
    }
    if tasks.current() == idle && idle_state != Some(State::Running) {
        return Some("idle is current but not running");
    }
    // The invariant is about *queue membership*, and `State::Ready` is only the
    // state field: a task can be marked Ready and not be on the queue, which is
    // precisely the corruption this whole check exists to catch. The queue is
    // not directly observable, but its emptiness is — and if idle is not
    // running, idle itself is queued, so something is always ready.
    //
    // This line was added because the model found the first version too weak:
    // a mutation that stops requeueing idle on yield left `state(IDLE)` at
    // `Ready` and passed. The model did not find a kernel bug; it found that
    // the property being asserted was not the property claimed.
    if tasks.current() != idle && !tasks.has_ready() {
        return Some("idle is not current and nothing is queued — idle left the run queue");
    }

    // 2. A slot that exited is free. Running one would be running a task whose
    //    stack has been handed back.
    if tasks.state(tasks.current()) == Some(State::Empty) {
        return Some("an empty slot is current");
    }

    // 3. One core, cooperative: two tasks in Running is a state the machine
    //    must not be able to reach.
    let running = (0..SLOTS)
        .filter(|&i| {
            tasks
                .live_id(i)
                .and_then(|id| tasks.state(id))
                .is_some_and(|s| s == State::Running)
        })
        .count();
    if running != 1 {
        return Some("not exactly one task is Running");
    }

    // 4. A switch to oneself is a context switch that costs and does nothing;
    //    the caller would swap a stack for itself.
    if let Some(Decision::Switch { from, to, .. }) = decision
        && from == to
    {
        return Some("Decision::Switch names the same task twice");
    }

    // 5. ADR-0062: an id from a slot's previous tenancy is invisible — it
    //    names no state, whatever the slot holds now.
    for slot in 0..SLOTS {
        if tasks.state(stale_id_for(tasks, slot)).is_some() {
            return Some("a stale id still names a state (ADR-0062)");
        }
    }

    None
}

/// Replay one sequence, returning the step index and reason of the first
/// violation.
fn replay(seq: &[Op]) -> Option<(usize, &'static str)> {
    let mut tasks = Tasks::<SLOTS>::new();
    tasks.start();
    if let Some(reason) = violation(&tasks, None) {
        return Some((0, reason));
    }
    for (i, &op) in seq.iter().enumerate() {
        let (decision, op_violation) = apply(&mut tasks, op);
        if let Some(reason) = op_violation.or_else(|| violation(&tasks, decision)) {
            return Some((i + 1, reason));
        }
    }
    None
}

#[cfg_attr(
    miri,
    ignore = "interpreted, the walk takes hours; this crate's only unsafe is ring.rs, which the unit tests already put under Miri"
)]
#[test]
fn every_sequence_up_to_depth_holds_the_scheduler_invariants() {
    let alphabet = ALPHABET.len();
    let mut sequences = 0u64;
    let mut buf = [Op::Admit; DEPTH];

    // Enumerate every sequence of every length up to DEPTH as a number in base
    // `alphabet`. Shorter sequences first, so the first counter-example found
    // is also among the shortest — which is the one worth reading.
    for len in 0..=DEPTH {
        let total = (alphabet as u64).pow(len as u32);
        for n in 0..total {
            let mut rest = n;
            for slot in buf.iter_mut().take(len) {
                *slot = ALPHABET[(rest % alphabet as u64) as usize];
                rest /= alphabet as u64;
            }
            let seq = &buf[..len];
            sequences += 1;
            if let Some((step, reason)) = replay(seq) {
                panic!(
                    "invariant broken after step {step}: {reason}\n\
                     counter-example ({len} ops): {seq:?}\n\
                     replay it by applying those in order to a fresh Tasks::<{SLOTS}>",
                );
            }
        }
    }

    // Printed so the bound is visible in the test output rather than only in
    // this file: a number that silently shrinks is a check that silently stops
    // checking.
    println!(
        "model_sched: {sequences} sequences over {alphabet} operations, \
         depth ≤ {DEPTH}, Tasks<{SLOTS}> — all invariants held"
    );
}

/// The search is only worth its runtime if it can reach the interesting states.
/// This asserts the walk actually gets there, rather than passing because every
/// sequence died early on a `Stay`.
#[cfg_attr(
    miri,
    ignore = "interpreted, the walk takes hours; this crate's only unsafe is ring.rs, which the unit tests already put under Miri"
)]
#[test]
fn the_search_reaches_the_states_it_claims_to_cover() {
    let mut saw_switch = false;
    let mut saw_exit_release = false;
    let mut saw_blocked = false;
    let mut saw_full_table = false;
    let mut saw_reused_slot = false;

    let alphabet = ALPHABET.len();
    let mut buf = [Op::Admit; DEPTH];
    for len in 0..=DEPTH {
        let total = (alphabet as u64).pow(len as u32);
        for n in 0..total {
            let mut rest = n;
            for slot in buf.iter_mut().take(len) {
                *slot = ALPHABET[(rest % alphabet as u64) as usize];
                rest /= alphabet as u64;
            }
            let mut tasks = Tasks::<SLOTS>::new();
            tasks.start();
            for &op in &buf[..len] {
                if let (Some(Decision::Switch { release, .. }), _) = apply(&mut tasks, op) {
                    saw_switch = true;
                    saw_exit_release |= release.is_some();
                }
            }
            for i in 0..SLOTS {
                let state = tasks.live_id(i).and_then(|id| tasks.state(id));
                saw_blocked |= state == Some(State::Blocked);
                saw_reused_slot |= tasks.live_id(i).is_some_and(|id| id.epoch() > 0);
            }
            saw_full_table |= (0..SLOTS).all(|i| tasks.live_id(i).is_some());
        }
    }

    assert!(saw_switch, "no sequence ever produced a context switch");
    assert!(saw_blocked, "no sequence ever parked a task");
    assert!(saw_full_table, "no sequence ever filled the task table");
    assert!(
        saw_reused_slot,
        "no sequence ever re-admitted into an exited slot — the stale-id \
         invariant (ADR-0062) was never exercised against a live successor"
    );
    assert!(
        saw_exit_release,
        "no sequence ever produced an exit that had to release a parked stack — \
         the path `Decision::Switch{{release: Some(_)}}` was never taken, so any \
         invariant about it is untested"
    );
}
