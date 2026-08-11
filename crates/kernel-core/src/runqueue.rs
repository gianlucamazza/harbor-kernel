//! Fixed-capacity FIFO ready queue for cooperative scheduling.
//!
//! This is the pure arithmetic half of [ADR-0006](../../../docs/adr/0006-cooperative-execution-model.md):
//! order ready tasks, nothing else. The kernel crate owns TCBs, stacks, the
//! voluntary switch, and the idle task. Priorities, blocking, and preemption
//! are deliberately absent.
//!
//! # Contract
//!
//! - The **running** task is not on the queue.
//! - A voluntary yield that keeps the current task runnable **enqueues** it,
//!   then **dequeues** the next; an empty queue after that means run idle.
//! - Capacity is fixed at compile time. Enqueue fails rather than overwriting.
//! - Identities are opaque integers. This module does not detect double-enqueue
//!   of the same id — that is a kernel bug, not a queue concern.

/// Task identity: a slot plus the epoch of its tenancy (ADR-0062).
///
/// Slots are reused after exit; the epoch is what makes a reference to the
/// previous tenant refusable instead of silently naming the next one. Only
/// [`crate::tasks::Tasks::admit`] mints a live id; equality is over both
/// fields, so a stale id never compares equal to the slot's current tenant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaskId {
    slot: u16,
    epoch: u16,
}

impl TaskId {
    /// Construct an id naming `slot` at `epoch`.
    ///
    /// Constructing an id does not make it live — validation against the
    /// slot's current epoch happens in `Tasks::state`/`Tasks::wake`.
    pub const fn new(slot: u16, epoch: u16) -> Self {
        Self { slot, epoch }
    }

    /// The slot index, for array addressing.
    #[inline]
    pub const fn slot(self) -> usize {
        self.slot as usize
    }

    /// The tenancy epoch this id was minted under.
    #[inline]
    pub const fn epoch(self) -> u16 {
        self.epoch
    }

    /// Pack for transport (wake tokens, atomics): `epoch << 16 | slot`.
    ///
    /// Transport, not authority — an unpacked stale id fails validation like
    /// any other.
    #[inline]
    pub const fn to_raw(self) -> u32 {
        ((self.epoch as u32) << 16) | self.slot as u32
    }

    /// Unpack a [`Self::to_raw`] value.
    #[inline]
    pub const fn from_raw(raw: u32) -> Self {
        Self {
            slot: raw as u16,
            epoch: (raw >> 16) as u16,
        }
    }
}

/// The ready queue could not accept another task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Full;

/// Fixed-capacity FIFO of ready [`TaskId`]s.
///
/// `CAP` is the maximum number of ready tasks that may sit on the queue at
/// once. Because yield enqueues the current task before dequeueing the next,
/// that peak is "every other task ready, plus the yielder" — size the constant
/// for the maximum number of concurrent tasks the kernel will ever create.
#[derive(Clone, Debug)]
pub struct RunQueue<const CAP: usize> {
    slots: [TaskId; CAP],
    /// Index of the next id to dequeue.
    head: usize,
    /// How many ids are currently stored.
    len: usize,
}

impl<const CAP: usize> RunQueue<CAP> {
    /// Empty queue. `CAP` may be zero; every enqueue then fails.
    pub const fn new() -> Self {
        Self {
            // The idle-shaped id is only a placeholder for unused slots; `len` is the
            // sole occupancy truth.
            slots: [TaskId::new(0, 0); CAP],
            head: 0,
            len: 0,
        }
    }

    /// Compile-time capacity.
    #[inline]
    pub const fn capacity(&self) -> usize {
        CAP
    }

    /// Number of ready tasks currently queued.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub const fn is_full(&self) -> bool {
        self.len == CAP
    }

    /// Append a ready task at the tail.
    ///
    /// Returns [`Full`] without modifying the queue when capacity is exhausted.
    pub fn enqueue(&mut self, id: TaskId) -> Result<(), Full> {
        if self.is_full() {
            return Err(Full);
        }
        // tail = (head + len) mod CAP; CAP == 0 is already handled by is_full.
        let tail = (self.head + self.len) % CAP;
        self.slots[tail] = id;
        self.len += 1;
        Ok(())
    }

    /// Remove and return the task at the head, or `None` if empty.
    pub fn dequeue(&mut self) -> Option<TaskId> {
        if self.is_empty() {
            return None;
        }
        let id = self.slots[self.head];
        self.head = (self.head + 1) % CAP;
        self.len -= 1;
        Some(id)
    }

    /// Head of the ready queue without removing it (ADR-0082 steal peeks).
    #[inline]
    pub fn front(&self) -> Option<TaskId> {
        if self.is_empty() {
            None
        } else {
            Some(self.slots[self.head])
        }
    }

    /// Call `f` for each ready id from head to tail (ADR-0082 steal probe).
    #[inline]
    pub fn for_each_ready(&self, mut f: impl FnMut(TaskId)) {
        for i in 0..self.len {
            f(self.slots[(self.head + i) % CAP]);
        }
    }

    /// Select who runs after a voluntary yield.
    ///
    /// - `requeue = Some(id)`: the current task remains runnable — enqueue it,
    ///   then dequeue the next (round-robin).
    /// - `requeue = None`: the current task left the ready set (exit); only
    ///   dequeue.
    ///
    /// Returns:
    /// - `Ok(Some(next))` — switch to `next` (may equal the yielder when it was
    ///   alone and requeued).
    /// - `Ok(None)` — ready set empty; the kernel should run idle.
    /// - `Err(Full)` — requeue was requested but the queue had no free slot.
    ///   The queue is unchanged; the caller still holds the running task.
    pub fn after_yield(&mut self, requeue: Option<TaskId>) -> Result<Option<TaskId>, Full> {
        if let Some(id) = requeue {
            self.enqueue(id)?;
        }
        Ok(self.dequeue())
    }
}

impl<const CAP: usize> Default for RunQueue<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u16) -> TaskId {
        TaskId::new(n, 0)
    }

    #[test]
    fn for_each_ready_walks_head_to_tail_including_across_the_wrap() {
        // The steal probe (ADR-0082) reads the queue through this, and its
        // caller only asks "is there one?" — so the ring arithmetic here was
        // never observed in order until the seventh mutation run reported
        // `(head + i) % CAP` untested.
        let mut q = RunQueue::<4>::new();
        for n in 1..=3 {
            q.enqueue(id(n)).unwrap();
        }
        let mut seen = Vec::new();
        q.for_each_ready(|t| seen.push(t));
        assert_eq!(seen, vec![id(1), id(2), id(3)], "head to tail, in order");

        // Push head past the end so the walk has to wrap.
        assert_eq!(q.dequeue(), Some(id(1)));
        assert_eq!(q.dequeue(), Some(id(2)));
        q.enqueue(id(4)).unwrap();
        q.enqueue(id(5)).unwrap();
        seen.clear();
        q.for_each_ready(|t| seen.push(t));
        assert_eq!(
            seen,
            vec![id(3), id(4), id(5)],
            "the walk follows the ring, not the slot order"
        );

        // An empty queue calls back for nothing at all.
        let empty = RunQueue::<4>::new();
        let mut count = 0;
        empty.for_each_ready(|_| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn raw_transport_round_trips_slot_and_epoch() {
        // ADR-0062: the packed form is transport for wake tokens and atomics.
        // Exact layout asserted (epoch high, slot low) — a swapped or collapsed
        // pack would alias distinct tasks in the wake queue.
        let id = TaskId::new(3, 2);
        assert_eq!(id.to_raw(), 0x0002_0003);
        let back = TaskId::from_raw(id.to_raw());
        assert_eq!(back, id);
        assert_eq!(back.slot(), 3);
        assert_eq!(back.epoch(), 2);
        assert_eq!(TaskId::from_raw(0x0001_0000), TaskId::new(0, 1));
    }

    #[test]
    fn capacity_reports_the_const() {
        assert_eq!(RunQueue::<4>::new().capacity(), 4);
        assert_eq!(RunQueue::<0>::new().capacity(), 0);
    }

    #[test]
    fn empty_queue_dequeues_nothing() {
        let mut q = RunQueue::<4>::new();
        assert!(q.is_empty());
        assert_eq!(q.dequeue(), None);
    }

    #[test]
    fn enqueue_dequeue_is_fifo() {
        let mut q = RunQueue::<4>::new();
        q.enqueue(id(1)).unwrap();
        q.enqueue(id(2)).unwrap();
        q.enqueue(id(3)).unwrap();
        assert_eq!(q.dequeue(), Some(id(1)));
        assert_eq!(q.dequeue(), Some(id(2)));
        assert_eq!(q.dequeue(), Some(id(3)));
        assert_eq!(q.dequeue(), None);
    }

    #[test]
    fn full_enqueue_is_refused_without_dropping() {
        let mut q = RunQueue::<2>::new();
        q.enqueue(id(1)).unwrap();
        q.enqueue(id(2)).unwrap();
        assert_eq!(q.enqueue(id(3)), Err(Full));
        assert_eq!(q.len(), 2);
        assert_eq!(q.dequeue(), Some(id(1)));
        assert_eq!(q.dequeue(), Some(id(2)));
    }

    #[test]
    fn zero_capacity_never_accepts() {
        let mut q = RunQueue::<0>::new();
        assert!(q.is_full());
        assert_eq!(q.enqueue(id(1)), Err(Full));
        assert_eq!(q.dequeue(), None);
    }

    #[test]
    fn ring_wraps_head_and_tail() {
        let mut q = RunQueue::<3>::new();
        // Fill, drain one, fill again so head and tail both wrap.
        q.enqueue(id(10)).unwrap();
        q.enqueue(id(11)).unwrap();
        q.enqueue(id(12)).unwrap();
        assert_eq!(q.dequeue(), Some(id(10)));
        q.enqueue(id(13)).unwrap();
        assert_eq!(q.dequeue(), Some(id(11)));
        assert_eq!(q.dequeue(), Some(id(12)));
        assert_eq!(q.dequeue(), Some(id(13)));
        assert!(q.is_empty());
    }

    #[test]
    fn yield_with_others_ready_rotates_round_robin() {
        // Running A; B and C ready. Yield keeps A runnable → B runs, A at tail.
        let mut q = RunQueue::<4>::new();
        q.enqueue(id(2)).unwrap(); // B
        q.enqueue(id(3)).unwrap(); // C
        let next = q.after_yield(Some(id(1))).unwrap();
        assert_eq!(next, Some(id(2)));
        assert_eq!(q.dequeue(), Some(id(3)));
        assert_eq!(q.dequeue(), Some(id(1)));
        assert!(q.is_empty());
    }

    #[test]
    fn yield_alone_and_requeued_runs_itself_again() {
        let mut q = RunQueue::<2>::new();
        let next = q.after_yield(Some(id(7))).unwrap();
        assert_eq!(next, Some(id(7)));
        assert!(q.is_empty());
    }

    #[test]
    fn yield_without_requeue_on_empty_means_idle() {
        let mut q = RunQueue::<2>::new();
        let next = q.after_yield(None).unwrap();
        assert_eq!(next, None);
    }

    #[test]
    fn exit_leaves_the_next_ready_task() {
        let mut q = RunQueue::<4>::new();
        q.enqueue(id(2)).unwrap();
        q.enqueue(id(3)).unwrap();
        // Task 1 exits: do not requeue.
        let next = q.after_yield(None).unwrap();
        assert_eq!(next, Some(id(2)));
        assert_eq!(q.dequeue(), Some(id(3)));
    }

    #[test]
    fn yield_requeue_on_full_queue_is_refused_and_queue_untouched() {
        let mut q = RunQueue::<2>::new();
        q.enqueue(id(2)).unwrap();
        q.enqueue(id(3)).unwrap();
        // Running 1; ready set already at capacity — cannot requeue.
        assert_eq!(q.after_yield(Some(id(1))), Err(Full));
        assert_eq!(q.len(), 2);
        assert_eq!(q.dequeue(), Some(id(2)));
        assert_eq!(q.dequeue(), Some(id(3)));
    }

    #[test]
    fn two_tasks_alternating_yields_interleave() {
        // Pure arithmetic stand-in for the M3 console interleaving done-when:
        // A and B keep yielding; the selected runner alternates.
        let mut q = RunQueue::<2>::new();
        let mut running = id(1);
        q.enqueue(id(2)).unwrap();

        let mut order = [id(0); 6];
        for slot in &mut order {
            *slot = running;
            running = q
                .after_yield(Some(running))
                .unwrap()
                .expect("peer stays ready");
        }

        assert_eq!(
            order,
            [id(1), id(2), id(1), id(2), id(1), id(2)],
            "cooperative RR must alternate when both requeue"
        );
    }
}
