//! Task states and the decision half of a cooperative switch (ADR-0006).
//!
//! [`RunQueue`] answers *which task runs next*. This answers the rest: what
//! happens to the one leaving, whether a switch is worth making, and whose
//! stack is now safe to release. It decides and does not act — the caller
//! performs the context switch and frees the memory, which is what keeps this
//! testable on a host with no MMU.
//!
//! # The stack a task cannot free
//!
//! A task cannot release the stack its own `SP` points into, so an exit parks
//! it and someone else collects it. There is one slot, and the invariant is
//! that it is always emptied before the next exit can fill it: [`Tasks::collect`]
//! runs after every context switch **and** on a task's first entry.
//!
//! The first-entry half was missing. An exit followed by a never-yet-run task
//! resumed at the trampoline, which did not collect, and the next exit
//! overwrote the slot — dropping a stack whose `Drop` is deliberately a no-op,
//! leaking it with no counter moving. [`Tasks::overwrites`] now counts that,
//! and the tests below drive the exact ordering, which nothing in a boot does.

use crate::runqueue::{RunQueue, TaskId};

/// Lifecycle of one task slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Free slot. Never reused before an exit clears it.
    Empty,
    /// Runnable and on the queue.
    Ready,
    /// Currently executing.
    Running,
    /// Parked on IPC or similar; not on the queue.
    Blocked,
}

/// Why the current task is giving up the CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    /// Voluntary yield: requeue and take the next.
    Yield,
    /// Involuntary yield (ADR-0064): the quantum expired and the safe-point
    /// consumer is rotating the current task. Same rotation as [`Switch::Yield`];
    /// idle is never rotated by it.
    Preempt,
    /// Park until woken. Idle may not block.
    Block,
    /// Terminate. Idle may not exit.
    Exit,
}

/// What the caller should do about a [`Tasks::switch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Keep running the current task; no context switch.
    Stay,
    /// Switch stacks. `release` names a task whose stack was already parked
    /// when this exit arrived — it belongs to nobody running, so the caller may
    /// free it immediately.
    Switch {
        from: TaskId,
        to: TaskId,
        release: Option<TaskId>,
    },
}

/// Fixed table of task slots plus the ready queue.
pub struct Tasks<const N: usize> {
    states: [State; N],
    /// Tenancy epoch per slot (ADR-0062). Bumped on exit; `admit` mints ids
    /// carrying it, and `state`/`wake` refuse an id whose epoch has moved on.
    epochs: [u16; N],
    queue: RunQueue<N>,
    current: TaskId,
    parked: Option<TaskId>,
    overwrites: u32,
    /// Successful entries into [`State::Blocked`] (ADR-0024).
    block_events: u32,
}

impl<const N: usize> Default for Tasks<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Tasks<N> {
    /// Slot 0 is idle: it runs on the bootstrap stack and is never queued.
    pub const IDLE: TaskId = TaskId::new(0, 0);

    pub const fn new() -> Self {
        Self {
            states: [State::Empty; N],
            epochs: [0; N],
            queue: RunQueue::new(),
            current: Self::IDLE,
            parked: None,
            overwrites: 0,
            block_events: 0,
        }
    }

    /// Claim idle as the running task. Call once, before any spawn.
    pub fn start(&mut self) {
        self.states[Self::IDLE.slot()] = State::Running;
        self.current = Self::IDLE;
    }

    /// The running task.
    #[inline]
    pub const fn current(&self) -> TaskId {
        self.current
    }

    /// The slot's state, or `None` for an unknown **or stale** id
    /// (ADR-0062): an epoch mismatch means the task this id named is gone,
    /// whatever now occupies the slot.
    #[inline]
    pub fn state(&self, id: TaskId) -> Option<State> {
        if self.epochs.get(id.slot()) != Some(&id.epoch()) {
            return None;
        }
        self.states.get(id.slot()).copied()
    }

    /// The live id currently occupying `slot`, or `None` if the slot is
    /// `Empty`. The one way to name a task by slot (iteration sites); it
    /// carries the current epoch, so the result validates everywhere.
    #[inline]
    pub fn live_id(&self, slot: usize) -> Option<TaskId> {
        match self.states.get(slot) {
            Some(State::Empty) | None => None,
            Some(_) => Some(TaskId::new(slot as u16, self.epochs[slot])),
        }
    }

    #[inline]
    pub fn has_ready(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Exits that found a stack still parked from an earlier exit.
    ///
    /// Must stay zero: every path onto another stack collects first. A non-zero
    /// value means one of them does not, and a stack was nearly lost.
    #[inline]
    pub const fn overwrites(&self) -> u32 {
        self.overwrites
    }

    /// How many task slots are currently [`State::Blocked`] (ADR-0024).
    ///
    /// Includes intentional waiters (e.g. the console server on an empty
    /// mailbox). Does not free them.
    #[inline]
    pub fn blocked_count(&self) -> u32 {
        self.states.iter().filter(|s| **s == State::Blocked).count() as u32
    }

    /// How many times a task has successfully entered [`State::Blocked`].
    #[inline]
    pub const fn block_events(&self) -> u32 {
        self.block_events
    }

    /// Take the free slot for a new task, marking it `Ready` and queueing it.
    ///
    /// `None` when every slot is taken, or when the queue refuses — in which
    /// case the slot is left `Empty` rather than half-admitted.
    pub fn admit(&mut self) -> Option<TaskId> {
        // A slot whose stack is still parked is not free, however `Empty` it
        // looks: the exit cleared the state and the memory is still waiting to
        // be collected. Reusing it would hand a new task the old stack's slot
        // and lose the pointer to it. The caller keeps the stack attached to
        // the slot, so this is the only thing standing between an exit and a
        // spawn that reuses it before anyone has run.
        let slot =
            self.states.iter().enumerate().position(|(i, s)| {
                *s == State::Empty && self.parked.is_none_or(|p| p.slot() != i)
            })?;
        let id = TaskId::new(slot as u16, self.epochs[slot]);
        if self.queue.enqueue(id).is_err() {
            return None;
        }
        self.states[slot] = State::Ready;
        Some(id)
    }

    /// Make a blocked task runnable again. `false` if it was not blocked —
    /// including a stale id (ADR-0062), which names a task that no longer
    /// exists.
    pub fn wake(&mut self, id: TaskId) -> bool {
        let idx = id.slot();
        if idx >= N || id == Self::IDLE || self.epochs[idx] != id.epoch() {
            return false;
        }
        if self.states[idx] != State::Blocked {
            return false;
        }
        self.states[idx] = State::Ready;
        self.queue.enqueue(id).is_ok()
    }

    /// Collect the parked stack, if any. Call after every switch onto another
    /// stack, including a task's very first entry.
    pub fn collect(&mut self) -> Option<TaskId> {
        self.parked.take()
    }

    /// Decide what a `switch_with(kind)` should do.
    ///
    /// ```
    /// use kernel_core::tasks::{Decision, Switch, Tasks};
    ///
    /// let mut tasks: Tasks<4> = Tasks::new();
    /// tasks.start(); // idle claims slot 0
    ///
    /// let worker = tasks.admit().unwrap();
    /// match tasks.switch(Switch::Yield) {
    ///     Decision::Switch { to, .. } => assert_eq!(to, worker),
    ///     Decision::Stay => panic!("a ready task was waiting"),
    /// }
    ///
    /// // The worker exits. Its stack cannot be freed from its own stack, so
    /// // the decision parks it and whoever runs next collects it.
    /// assert!(matches!(tasks.switch(Switch::Exit), Decision::Switch { .. }));
    /// assert_eq!(tasks.collect(), Some(worker));
    /// ```
    pub fn switch(&mut self, kind: Switch) -> Decision {
        let current = self.current;
        let cur = current.slot();

        let requeue = match kind {
            Switch::Preempt if current == Self::IDLE => {
                // Never preempt idle into idle (ADR-0064): a stale flag taken
                // with idle current is a no-op, not a rotation.
                return Decision::Stay;
            }
            Switch::Yield | Switch::Preempt => {
                if self.states[cur] == State::Running {
                    self.states[cur] = State::Ready;
                }
                true
            }
            Switch::Block | Switch::Exit if current == Self::IDLE => {
                // Idle has nothing to fall back to: blocking or exiting it
                // stops the machine.
                return Decision::Stay;
            }
            Switch::Block => {
                self.states[cur] = State::Blocked;
                self.block_events = self.block_events.saturating_add(1);
                false
            }
            Switch::Exit => {
                self.states[cur] = State::Empty;
                // ADR-0062: the tenancy ends here. Every stored reference to
                // this task — task-caps, wake tokens, pending IRQ deliveries —
                // carries the old epoch and is refused from now on.
                self.epochs[cur] = self.epochs[cur].wrapping_add(1);
                false
            }
        };

        let next = match self.queue.after_yield(requeue.then_some(current)) {
            Ok(Some(id)) => id,
            // Nothing ready: fall back to idle unless we are already it.
            //
            // Unreachable while the guard above holds. Idle is always exactly
            // one of *current* or *queued*: it is popped when it starts running
            // and requeued whenever it yields, and it may neither block nor
            // exit. So a worker asking for the next task always finds at least
            // idle waiting, and `Ok(None)` only ever arrives when idle itself
            // is asking. Mutation testing reports this guard as untested, and
            // no test can honestly cover it — it is what keeps the scheduler
            // from running a task that is no longer ready if that invariant
            // ever changes.
            Ok(None) if current != Self::IDLE => Self::IDLE,
            _ => {
                // Either idle with an empty queue, or the queue refused the
                // requeue. The current task keeps running, and nothing about
                // the parked slot changes — an earlier exit's stack is still
                // waiting for whoever collects next.
                self.states[cur] = State::Running;
                return Decision::Stay;
            }
        };

        if next == current {
            self.states[cur] = State::Running;
            return Decision::Stay;
        }

        // Only now, with the switch certain, does the parked slot move. Doing
        // it earlier would leave a task that ends up staying with its stack
        // handed away, or drop an earlier exit's stack on a path that returns.
        let mut release = None;
        if kind == Switch::Exit
            && let Some(stale) = self.parked.replace(current)
        {
            // Not ours — its owner exited earlier — so the caller can free
            // it now. Counted because with both collection points in place
            // this cannot happen.
            self.overwrites += 1;
            release = Some(stale);
        }
        self.states[next.slot()] = State::Running;
        self.current = next;
        Decision::Switch {
            from: current,
            to: next,
            release,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Idle plus three slots — enough for the orderings that matter.
    type T = Tasks<4>;

    fn started() -> T {
        let mut t = T::new();
        t.start();
        t
    }

    #[test]
    fn idle_starts_running_and_is_never_queued() {
        let t = started();
        assert_eq!(t.current(), T::IDLE);
        assert_eq!(t.state(T::IDLE), Some(State::Running));
        assert!(!t.has_ready(), "idle does not queue itself");
    }

    #[test]
    fn idle_may_neither_block_nor_exit() {
        // It has nothing to fall back to: either would stop the machine.
        for kind in [Switch::Block, Switch::Exit] {
            let mut t = started();
            assert_eq!(t.switch(kind), Decision::Stay, "{kind:?}");
            assert_eq!(t.state(T::IDLE), Some(State::Running));
        }
    }

    #[test]
    fn admitting_past_the_last_slot_fails_without_disturbing_the_others() {
        let mut t = started();
        let ids: Vec<_> = (0..3).map(|_| t.admit().unwrap()).collect();
        assert_eq!(
            ids,
            vec![TaskId::new(1, 0), TaskId::new(2, 0), TaskId::new(3, 0)]
        );
        assert_eq!(t.admit(), None, "slot 0 is idle, so three is the maximum");
        for id in ids {
            assert_eq!(t.state(id), Some(State::Ready));
        }
    }

    #[test]
    fn a_yielding_task_is_marked_ready_and_not_left_running() {
        // Two tasks cannot both be `Running`. The state is what `admit` reads
        // to find a free slot and what the boot report prints, so leaving a
        // queued task marked `Running` is not cosmetic.
        let mut t = started();
        let a = t.admit().unwrap();
        t.switch(Switch::Yield); // idle → a
        assert_eq!(t.state(T::IDLE), Some(State::Ready), "idle yielded, queued");
        assert_eq!(t.state(a), Some(State::Running));
    }

    #[test]
    fn idle_refuses_to_block_even_with_somewhere_to_go() {
        // The guard reads `current == IDLE`, and with a ready task waiting the
        // difference is total: the real kernel stays on idle, while a scheduler
        // that let idle block would mark it `Blocked` with nothing left to
        // wake it — the machine stops the moment that task exits.
        let mut t = started();
        let _a = t.admit().unwrap();
        assert!(t.has_ready(), "somewhere to go, deliberately");

        assert!(matches!(t.switch(Switch::Block), Decision::Stay));
        assert_eq!(t.current(), T::IDLE);
        assert_eq!(t.state(T::IDLE), Some(State::Running));

        assert!(matches!(t.switch(Switch::Exit), Decision::Stay));
        assert_eq!(t.state(T::IDLE), Some(State::Running), "idle never exits");
    }

    #[test]
    fn has_ready_answers_both_ways() {
        // The idle loop calls this to decide between `WFI` and another round.
        // Hard-wired to `false` it would sleep with work waiting; hard-wired to
        // `true` it would spin. Only asserting the false case leaves half of it
        // uncovered.
        let mut t = started();
        assert!(!t.has_ready(), "nothing admitted yet");
        let _ = t.admit().unwrap();
        assert!(t.has_ready(), "a ready task is waiting");
    }

    #[test]
    fn a_worker_may_block_and_exit_where_idle_may_not() {
        // The guards are `current == IDLE`, and a test that only ever blocks
        // idle cannot tell them from `false`. Both sides, same assertions.
        let mut t = started();
        let a = t.admit().unwrap();
        t.switch(Switch::Yield); // → a
        assert_eq!(t.current(), a);
        assert!(
            matches!(t.switch(Switch::Block), Decision::Switch { .. }),
            "a worker blocking really switches away"
        );
        assert_eq!(t.state(a), Some(State::Blocked));

        let mut t = started();
        let b = t.admit().unwrap();
        t.switch(Switch::Yield); // → b
        assert!(
            matches!(t.switch(Switch::Exit), Decision::Switch { .. }),
            "a worker exiting really switches away"
        );
        assert_eq!(t.state(b), None, "the exited id is stale (ADR-0062)");
    }

    #[test]
    fn blocked_count_and_block_events_track_parks() {
        // ADR-0024: parks are visible. Idle block must not count (Stay).
        let mut t = started();
        assert_eq!(t.blocked_count(), 0);
        assert_eq!(t.block_events(), 0);
        assert!(matches!(t.switch(Switch::Block), Decision::Stay));
        assert_eq!(t.block_events(), 0, "idle must not accrue block events");

        let a = t.admit().unwrap();
        t.switch(Switch::Yield); // → a
        assert!(matches!(t.switch(Switch::Block), Decision::Switch { .. }));
        assert_eq!(t.blocked_count(), 1);
        assert_eq!(t.block_events(), 1);

        t.wake(a);
        assert_eq!(t.blocked_count(), 0);
        assert_eq!(t.block_events(), 1, "wake does not erase history");
    }

    #[test]
    fn a_worker_with_nothing_else_ready_falls_back_to_idle() {
        // The `Ok(None) if current != IDLE` arm. With the guard inverted a
        // worker would stay on itself forever after an exit, and with it always
        // true idle would try to fall back to itself.
        let mut t = started();
        let a = t.admit().unwrap();
        t.switch(Switch::Yield); // → a, queue now holds idle
        t.switch(Switch::Yield); // → idle, queue holds a
        assert_eq!(t.current(), T::IDLE);

        // Drain a out of the queue so nothing is ready but idle itself.
        t.switch(Switch::Yield); // → a
        assert_eq!(t.current(), a);
        assert!(
            matches!(t.switch(Switch::Exit), Decision::Switch { to, .. } if to == T::IDLE),
            "the last worker exits into idle"
        );
    }

    #[test]
    fn a_preempted_worker_rotates_like_a_yield() {
        // ADR-0064: same rotation as Yield — the preempted task goes to the
        // queue tail marked Ready, the head runs.
        let mut t = started();
        let a = t.admit().unwrap();
        let b = t.admit().unwrap();
        t.switch(Switch::Yield); // → a
        assert_eq!(t.current(), a);

        assert!(
            matches!(t.switch(Switch::Preempt), Decision::Switch { from, to, .. }
                if from == a && to == b),
            "the preempted worker hands the CPU to the next ready task"
        );
        assert_eq!(t.state(a), Some(State::Ready), "preempted, not blocked");
        assert_eq!(t.state(b), Some(State::Running));
    }

    #[test]
    fn preempting_idle_is_a_stay() {
        // Never idle → idle, even with a ready task waiting: preemption exists
        // to bound a running *worker*; idle reschedules through its own loop.
        let mut t = started();
        let _a = t.admit().unwrap();
        assert_eq!(t.switch(Switch::Preempt), Decision::Stay);
        assert_eq!(t.current(), T::IDLE);
        assert_eq!(t.state(T::IDLE), Some(State::Running));
    }

    #[test]
    fn a_preempted_worker_alone_rotates_through_idle() {
        // A worker always finds at least idle queued (idle is exactly one of
        // current or queued), so a lone spinner rotates into idle and comes
        // back next round — it is never wedged on itself.
        let mut t = started();
        let a = t.admit().unwrap();
        t.switch(Switch::Yield); // → a, queue holds idle
        assert_eq!(t.current(), a);
        assert!(matches!(t.switch(Switch::Preempt), Decision::Switch { to, .. } if to == T::IDLE));
        assert_eq!(t.state(a), Some(State::Ready), "back in the rotation");
    }

    #[test]
    fn a_yield_with_nothing_ready_stays_put() {
        let mut t = started();
        assert_eq!(t.switch(Switch::Yield), Decision::Stay);
        assert_eq!(t.current(), T::IDLE);
    }

    #[test]
    fn idle_takes_its_turn_in_the_rotation() {
        // Not obvious, and worth pinning: a yield requeues *whoever* yielded,
        // idle included, so the rotation is a → b → idle → a and not a → b → a.
        // The idle body is the console loop, which yields in its own loop, so
        // this is the live behaviour and not an artefact of the model.
        let mut t = started();
        let a = t.admit().unwrap();
        let b = t.admit().unwrap();

        let mut seen = Vec::new();
        for _ in 0..6 {
            match t.switch(Switch::Yield) {
                Decision::Switch { to, .. } => seen.push(to),
                Decision::Stay => seen.push(t.current()),
            }
        }
        assert_eq!(seen, vec![a, b, T::IDLE, a, b, T::IDLE]);
    }

    #[test]
    fn the_two_workers_alternate_with_each_other() {
        // What the M3 boot check reads as interleaved output: between any two
        // appearances of a there is exactly one b, whatever idle does.
        let mut t = started();
        let a = t.admit().unwrap();
        let b = t.admit().unwrap();

        let mut workers = Vec::new();
        for _ in 0..9 {
            if let Decision::Switch { to, .. } = t.switch(Switch::Yield)
                && to != T::IDLE
            {
                workers.push(to);
            }
        }
        assert_eq!(workers, vec![a, b, a, b, a, b]);
    }

    #[test]
    fn a_blocked_task_is_skipped_until_woken() {
        let mut t = started();
        let a = t.admit().unwrap();
        let b = t.admit().unwrap();
        t.switch(Switch::Yield); // → a
        assert_eq!(t.current(), a);

        t.switch(Switch::Block); // a parks
        assert_eq!(t.state(a), Some(State::Blocked));
        assert_eq!(t.current(), b, "b runs while a is blocked");

        t.switch(Switch::Yield);
        assert_ne!(t.current(), a, "a is not on the queue");

        assert!(t.wake(a));
        assert_eq!(t.state(a), Some(State::Ready));
        assert!(!t.wake(a), "waking a ready task changes nothing");
    }

    #[test]
    fn waking_idle_or_an_unknown_slot_is_refused() {
        let mut t = started();
        assert!(!t.wake(T::IDLE));
        assert!(!t.wake(TaskId::new(99, 0)));
    }

    #[test]
    fn an_exit_parks_its_stack_for_whoever_runs_next() {
        let mut t = started();
        let a = t.admit().unwrap();
        t.switch(Switch::Yield); // → a
        assert_eq!(
            t.switch(Switch::Exit),
            Decision::Switch {
                from: a,
                to: T::IDLE,
                release: None,
            }
        );
        assert_eq!(t.state(a), None, "the exited id is stale (ADR-0062)");
        assert_eq!(t.collect(), Some(a), "the stack is waiting to be freed");
        assert_eq!(t.collect(), None, "and only once");
    }

    #[test]
    fn an_exit_into_a_task_that_has_never_run_still_parks() {
        // The P0-2 ordering. The kernel resumes a first-run task at a
        // trampoline, which used to skip the collection point — so the stack
        // sat parked until the *next* exit overwrote it.
        let mut t = started();
        let a = t.admit().unwrap();
        let b = t.admit().unwrap();
        t.switch(Switch::Yield); // idle → a. b has never run.
        assert_eq!(t.current(), a);

        let d = t.switch(Switch::Exit);
        assert!(
            matches!(d, Decision::Switch { to, release: None, .. } if to == b),
            "a exits straight into b, which has never run: {d:?}"
        );
        assert_eq!(
            t.collect(),
            Some(a),
            "b's first entry must collect, or a's stack is stranded"
        );
    }

    #[test]
    fn skipping_a_collection_point_is_counted_not_silent() {
        // Drive the defect: exit, do *not* collect, exit again. The second exit
        // finds the slot taken. It is reported for release rather than dropped,
        // and the counter moves — which is what the boot check watches.
        let mut t = started();
        let a = t.admit().unwrap();
        let b = t.admit().unwrap();
        t.switch(Switch::Yield); // → a
        t.switch(Switch::Exit); // a parks, → b
        assert_eq!(t.overwrites(), 0);

        // b exits without anyone having collected a.
        let d = t.switch(Switch::Exit);
        assert!(
            matches!(d, Decision::Switch { release: Some(stranded), .. } if stranded == a),
            "the earlier stack is handed back rather than dropped: {d:?}"
        );
        assert_eq!(t.overwrites(), 1, "and the skip is counted");
        assert_eq!(t.collect(), Some(b), "b's own stack is now the parked one");
    }

    #[test]
    fn collecting_after_every_switch_keeps_the_counter_at_zero() {
        // The invariant the kernel maintains, driven for more exits than a boot
        // performs. Nothing here should ever raise `overwrites`.
        let mut t = started();
        for _ in 0..20 {
            let a = t.admit().unwrap();
            t.switch(Switch::Yield);
            if t.current() == a {
                t.switch(Switch::Exit);
                t.collect();
            }
        }
        assert_eq!(t.overwrites(), 0);
    }

    #[test]
    fn a_slot_whose_stack_is_still_parked_is_not_handed_out() {
        // The state says `Empty` and the memory says otherwise. Admitting into
        // it would attach a new stack to the slot and lose the old pointer.
        let mut t = Tasks::<2>::new();
        t.start();
        let a = t.admit().unwrap();
        t.switch(Switch::Yield); // → a
        t.switch(Switch::Exit); // a exits; its stack is parked
        assert_eq!(t.state(a), None, "the exited id is stale (ADR-0062)");
        assert_eq!(t.admit(), None, "the only free slot still owns a stack");
        assert_eq!(t.collect(), Some(a));
        let b = t.admit().expect("collected, so now it really is free");
        assert_eq!(b.slot(), a.slot(), "same slot");
        assert_ne!(b, a, "new tenancy, new identity (ADR-0062)");
    }

    #[test]
    fn a_stay_decision_leaves_an_earlier_exits_stack_parked() {
        // A yield that finds nothing ready must not disturb the slot: the stack
        // belongs to a task that already exited and is still waiting.
        let mut t = started();
        let a = t.admit().unwrap();
        t.switch(Switch::Yield); // → a
        t.switch(Switch::Exit); // a parks, → idle
        assert_eq!(
            t.switch(Switch::Yield),
            Decision::Stay,
            "idle, nothing ready"
        );
        assert_eq!(t.collect(), Some(a), "still there");
    }

    #[test]
    fn a_stale_id_is_invisible_after_exit() {
        // ADR-0062: exit bumps the slot's epoch, so the old id names nothing —
        // state says unknown, wake refuses — while the slot itself is reusable.
        let mut t = started();
        let a = t.admit().unwrap();
        t.switch(Switch::Yield); // → a
        t.switch(Switch::Block); // a parks, → idle
        t.wake(a);
        t.switch(Switch::Yield); // → a
        t.switch(Switch::Exit); // a exits
        t.collect();

        assert_eq!(t.state(a), None, "stale id is unknown, not Empty");
        assert!(!t.wake(a), "a stale id cannot wake anyone");

        let b = t.admit().unwrap();
        assert_eq!(b.slot(), a.slot(), "the slot is reused");
        assert_ne!(b, a, "but the identity is new");
        assert_eq!(t.state(a), None, "the old id does not name the new tenant");
        assert_eq!(t.state(b), Some(State::Ready));
    }

    #[test]
    fn live_id_names_the_current_tenant_only() {
        let mut t = started();
        let a = t.admit().unwrap();
        assert_eq!(t.live_id(a.slot()), Some(a));
        assert_eq!(t.live_id(0), Some(T::IDLE));
        assert_eq!(t.live_id(2), None, "empty slot has no live id");
        assert_eq!(t.live_id(99), None, "out of range");

        t.switch(Switch::Yield); // → a
        t.switch(Switch::Exit);
        t.collect();
        assert_eq!(t.live_id(a.slot()), None, "exited slot has no live id");
    }

    #[cfg_attr(
        miri,
        ignore = "65 k exit cycles interpreted take tens of minutes; the loop is pure arithmetic with no unsafe — this crate's only unsafe is ring.rs, which the unit tests already put under Miri"
    )]
    #[test]
    fn epoch_wraps_after_65536_exit_cycles() {
        // ADR-0062: the u16 epoch wraps after 65 536 exits on one slot, and the
        // original stale id validates again. This encodes the decided bound
        // (same shape as ADR-0057 §3) rather than pretending it is unbounded.
        let mut t = Tasks::<2>::new();
        t.start();
        let first = t.admit().unwrap();
        for _ in 0..65536 {
            t.switch(Switch::Yield); // → worker
            t.switch(Switch::Exit); // worker exits, → idle
            t.collect();
            t.admit().unwrap();
        }
        let current = t.live_id(first.slot()).unwrap();
        assert_eq!(current, first, "the epoch wrapped back to the first id");
        assert_eq!(t.state(first), Some(State::Ready));
    }
}
