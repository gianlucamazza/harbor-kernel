//! IRQ wait table (ADR-0028) — pure, host-tested.
//!
//! One waiter **per cookie**. A second arm on a busy cookie is refused; a task
//! may wait on only one cookie at a time. No "last arm wins".

/// How many concurrent IRQ waits the table can hold.
pub const MAX_WAITERS: usize = 8;

/// Task id space for pending bits (must cover the kernel's `MAX_TASKS`).
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
    tasks: [Option<u32>; MAX_WAITERS],
    /// Set when `signal` matches; closed by [`Self::take_pending`].
    pending: [bool; MAX_TASK_IDS],
}

impl WaitTable {
    pub const fn new() -> Self {
        Self {
            cookies: [0; MAX_WAITERS],
            tasks: [None; MAX_WAITERS],
            pending: [false; MAX_TASK_IDS],
        }
    }

    /// Arm `task` to wake when `cookie` is signalled.
    pub fn arm(&mut self, cookie: u32, task: u32) -> Result<(), ArmError> {
        if task as usize >= MAX_TASK_IDS {
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
    pub fn disarm_task(&mut self, task: u32) {
        for i in 0..MAX_WAITERS {
            if self.tasks[i] == Some(task) {
                self.tasks[i] = None;
            }
        }
    }

    /// IRQ path: if a waiter is armed for `cookie`, clear the slot, mark
    /// pending, and return the task token to enqueue for wake.
    pub fn signal(&mut self, cookie: u32) -> Option<u32> {
        for i in 0..MAX_WAITERS {
            if self.tasks[i].is_some() && self.cookies[i] == cookie {
                let task = self.tasks[i].take().unwrap();
                if (task as usize) < MAX_TASK_IDS {
                    self.pending[task as usize] = true;
                }
                return Some(task);
            }
        }
        None
    }

    /// Consume pending for `task`. True if an IRQ already posted.
    pub fn take_pending(&mut self, task: u32) -> bool {
        let i = task as usize;
        if i >= MAX_TASK_IDS {
            return false;
        }
        let p = self.pending[i];
        self.pending[i] = false;
        p
    }

    /// Whether `task` still has a pending delivery (non-consuming).
    pub fn is_pending(&self, task: u32) -> bool {
        let i = task as usize;
        i < MAX_TASK_IDS && self.pending[i]
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

    #[test]
    fn arm_signal_round_trip() {
        let mut t = WaitTable::new();
        t.arm(1, 3).unwrap();
        assert_eq!(t.signal(1), Some(3));
        assert!(t.take_pending(3));
        assert_eq!(t.signal(1), None);
    }

    #[test]
    fn cookie_busy_is_refused() {
        let mut t = WaitTable::new();
        t.arm(1, 2).unwrap();
        assert_eq!(t.arm(1, 4), Err(ArmError::CookieBusy));
    }

    #[test]
    fn task_busy_is_refused() {
        let mut t = WaitTable::new();
        t.arm(1, 2).unwrap();
        assert_eq!(t.arm(2, 2), Err(ArmError::TaskBusy));
    }

    #[test]
    fn two_cookies_two_waiters() {
        let mut t = WaitTable::new();
        t.arm(1, 2).unwrap();
        t.arm(2, 3).unwrap();
        assert_eq!(t.signal(2), Some(3));
        assert_eq!(t.signal(1), Some(2));
    }

    #[test]
    fn lost_wakeup_pending_before_block() {
        let mut t = WaitTable::new();
        t.arm(1, 5).unwrap();
        // IRQ fires before the task parks.
        assert_eq!(t.signal(1), Some(5));
        assert!(t.take_pending(5));
        // No second delivery.
        assert!(!t.take_pending(5));
    }

    #[test]
    fn disarm_clears_arm_without_pending() {
        let mut t = WaitTable::new();
        t.arm(1, 5).unwrap();
        t.disarm_task(5);
        assert_eq!(t.signal(1), None);
    }
}
