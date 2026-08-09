//! IRQ wait table (ADR-0028) — pure, host-tested.
//!
//! One waiter **per cookie**. A second arm on a busy cookie is refused; a task
//! may wait on only one cookie at a time. No "last arm wins".
//!
//! Waiters and pending marks carry the full [`TaskId`] (ADR-0062): a delivery
//! posted for a task that has since exited is not consumable by the slot's
//! next tenant.

use crate::runqueue::TaskId;

/// How many concurrent IRQ waits the table can hold.
pub const MAX_WAITERS: usize = 8;

/// Task slot space for pending marks (must cover the kernel's `MAX_TASKS`).
pub const MAX_TASK_IDS: usize = 64;

/// Why [`WaitTable::arm`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmError {
    /// `task` is out of range for the pending bitmap.
    BadTask,
    /// This cookie already has a waiter.
    CookieBusy,
    /// This task is already armed on some cookie.
    TaskBusy,
    /// Table full (distinct cookies).
    Full,
}

/// Pure wait table: arm on the voluntary path, signal from IRQ logic.
#[derive(Clone, Debug)]
pub struct WaitTable {
    /// Parallel arrays: `cookies[i]` is live when `tasks[i] != None`.
    cookies: [u32; MAX_WAITERS],
    tasks: [Option<TaskId>; MAX_WAITERS],
    /// Set when `signal` matches; closed by [`Self::take_pending`]. Indexed by
    /// slot but holding the full id, so only the epoch it was posted for can
    /// consume it (ADR-0062).
    pending: [Option<TaskId>; MAX_TASK_IDS],
}

impl WaitTable {
    pub const fn new() -> Self {
        Self {
            cookies: [0; MAX_WAITERS],
            tasks: [None; MAX_WAITERS],
            pending: [None; MAX_TASK_IDS],
        }
    }

    /// Arm `task` to wake when `cookie` is signalled.
    pub fn arm(&mut self, cookie: u32, task: TaskId) -> Result<(), ArmError> {
        if task.slot() >= MAX_TASK_IDS {
            return Err(ArmError::BadTask);
        }
        for i in 0..MAX_WAITERS {
            if let Some(t) = self.tasks[i] {
                if self.cookies[i] == cookie {
                    return Err(ArmError::CookieBusy);
                }
                if t == task {
                    return Err(ArmError::TaskBusy);
                }
            }
        }
        for i in 0..MAX_WAITERS {
            if self.tasks[i].is_none() {
                self.cookies[i] = cookie;
                self.tasks[i] = Some(task);
                return Ok(());
            }
        }
        Err(ArmError::Full)
    }

    /// Drop any arm for `task` (voluntary path after wake or cancel).
    pub fn disarm_task(&mut self, task: TaskId) {
        for i in 0..MAX_WAITERS {
            if self.tasks[i] == Some(task) {
                self.tasks[i] = None;
            }
        }
    }

    /// IRQ path: if a waiter is armed for `cookie`, clear the slot, mark
    /// pending, and return the task token to enqueue for wake.
    pub fn signal(&mut self, cookie: u32) -> Option<TaskId> {
        for i in 0..MAX_WAITERS {
            if self.tasks[i].is_some() && self.cookies[i] == cookie {
                let task = self.tasks[i].take().unwrap();
                if task.slot() < MAX_TASK_IDS {
                    self.pending[task.slot()] = Some(task);
                }
                return Some(task);
            }
        }
        None
    }

    /// Consume pending for `task`. True if an IRQ already posted **for this
    /// id** — a mark left for the slot's previous tenant is not consumable
    /// (ADR-0062), and is cleared only when its own epoch asks.
    pub fn take_pending(&mut self, task: TaskId) -> bool {
        let i = task.slot();
        if i >= MAX_TASK_IDS || self.pending[i] != Some(task) {
            return false;
        }
        self.pending[i] = None;
        true
    }

    /// Whether `task` still has a pending delivery (non-consuming).
    pub fn is_pending(&self, task: TaskId) -> bool {
        let i = task.slot();
        i < MAX_TASK_IDS && self.pending[i] == Some(task)
    }
}

impl Default for WaitTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(n: u16) -> TaskId {
        TaskId::new(n, 0)
    }

    #[test]
    fn arm_signal_round_trip() {
        let mut t = WaitTable::new();
        t.arm(1, tid(3)).unwrap();
        assert_eq!(t.signal(1), Some(tid(3)));
        assert!(t.take_pending(tid(3)));
        assert_eq!(t.signal(1), None);
    }

    #[test]
    fn cookie_busy_is_refused() {
        let mut t = WaitTable::new();
        t.arm(1, tid(2)).unwrap();
        assert_eq!(t.arm(1, tid(4)), Err(ArmError::CookieBusy));
    }

    #[test]
    fn task_busy_is_refused() {
        let mut t = WaitTable::new();
        t.arm(1, tid(2)).unwrap();
        assert_eq!(t.arm(2, tid(2)), Err(ArmError::TaskBusy));
    }

    #[test]
    fn two_cookies_two_waiters() {
        let mut t = WaitTable::new();
        t.arm(1, tid(2)).unwrap();
        t.arm(2, tid(3)).unwrap();
        assert_eq!(t.signal(2), Some(tid(3)));
        assert_eq!(t.signal(1), Some(tid(2)));
    }

    #[test]
    fn lost_wakeup_pending_before_block() {
        let mut t = WaitTable::new();
        t.arm(1, tid(5)).unwrap();
        // IRQ fires before the task parks.
        assert_eq!(t.signal(1), Some(tid(5)));
        assert!(t.take_pending(tid(5)));
        // No second delivery.
        assert!(!t.take_pending(tid(5)));
    }

    #[test]
    fn disarm_clears_arm_without_pending() {
        let mut t = WaitTable::new();
        t.arm(1, tid(5)).unwrap();
        t.disarm_task(tid(5));
        assert_eq!(t.signal(1), None);
    }

    #[test]
    fn an_out_of_range_slot_is_never_pending() {
        // The bound is the claim: one past it must answer false, not index the
        // array (which is what the `<` → `<=` mutant would do).
        let t = WaitTable::new();
        assert!(!t.is_pending(TaskId::new(MAX_TASK_IDS as u16, 0)));
    }

    #[test]
    fn a_pending_mark_is_not_consumable_by_the_slots_next_tenant() {
        // ADR-0062: the mark carries the full id. The next tenant of the same
        // slot (new epoch) must neither observe nor consume it — and the stale
        // mark does not linger to satisfy a later wait either.
        let mut t = WaitTable::new();
        let old = TaskId::new(5, 0);
        let new = TaskId::new(5, 1);
        t.arm(1, old).unwrap();
        assert_eq!(t.signal(1), Some(old));
        assert!(!t.take_pending(new), "new tenant cannot consume");
        assert!(!t.is_pending(new));
        assert!(t.is_pending(old));
        assert!(t.take_pending(old), "the posted epoch still can");
    }
}
