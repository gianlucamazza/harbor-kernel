//! Park deadline table (ADR-0040 / K2 timeout) — pure, host-tested.
//!
//! Absolute tick deadlines for blocked waits. The kernel façade polls with
//! `time::ticks()` and cancels expired tasks on the voluntary path.

use crate::runqueue::TaskId;

/// Concurrent armed park deadlines.
pub const MAX_ARMED: usize = 16;

/// Why [`Table::arm`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmError {
    /// Table full and this task was not already armed.
    Full,
}

#[derive(Clone, Copy)]
struct Entry {
    live: bool,
    task: TaskId,
    deadline: u64,
}

impl Entry {
    const EMPTY: Self = Self {
        live: false,
        task: TaskId(0),
        deadline: 0,
    };
}

/// Pure park-deadline table.
#[derive(Clone)]
pub struct Table {
    entries: [Entry; MAX_ARMED],
}

impl Table {
    pub const fn new() -> Self {
        Self {
            entries: [Entry::EMPTY; MAX_ARMED],
        }
    }

    /// Arm or replace the deadline for `task`.
    pub fn arm(&mut self, task: TaskId, deadline: u64) -> Result<(), ArmError> {
        if let Some(i) = self.find(task) {
            self.entries[i].deadline = deadline;
            self.entries[i].live = true;
            return Ok(());
        }
        for e in &mut self.entries {
            if !e.live {
                e.live = true;
                e.task = task;
                e.deadline = deadline;
                return Ok(());
            }
        }
        Err(ArmError::Full)
    }

    /// Clear any deadline for `task`.
    pub fn disarm(&mut self, task: TaskId) {
        if let Some(i) = self.find(task) {
            self.entries[i] = Entry::EMPTY;
        }
    }

    /// Remove and return every task whose deadline is `<= now`.
    ///
    /// `out` is filled from the start; return value is how many were written.
    pub fn poll(&mut self, now: u64, out: &mut [TaskId; MAX_ARMED]) -> usize {
        let mut n = 0usize;
        for e in &mut self.entries {
            if e.live && e.deadline <= now {
                if n < MAX_ARMED {
                    out[n] = e.task;
                    n += 1;
                }
                *e = Entry::EMPTY;
            }
        }
        n
    }

    fn find(&self, task: TaskId) -> Option<usize> {
        self.entries.iter().position(|e| e.live && e.task == task)
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_poll_expires() {
        let mut t = Table::new();
        t.arm(TaskId(3), 10).unwrap();
        let mut out = [TaskId(0); MAX_ARMED];
        assert_eq!(t.poll(9, &mut out), 0);
        assert_eq!(t.poll(10, &mut out), 1);
        assert_eq!(out[0], TaskId(3));
        // Already cleared.
        assert_eq!(t.poll(100, &mut out), 0);
    }

    #[test]
    fn disarm_prevents_expire() {
        let mut t = Table::new();
        t.arm(TaskId(1), 5).unwrap();
        t.disarm(TaskId(1));
        let mut out = [TaskId(0); MAX_ARMED];
        assert_eq!(t.poll(5, &mut out), 0);
    }

    #[test]
    fn rearm_replaces_deadline() {
        let mut t = Table::new();
        t.arm(TaskId(2), 5).unwrap();
        t.arm(TaskId(2), 50).unwrap();
        let mut out = [TaskId(0); MAX_ARMED];
        assert_eq!(t.poll(5, &mut out), 0);
        assert_eq!(t.poll(50, &mut out), 1);
        assert_eq!(out[0], TaskId(2));
    }

    #[test]
    fn full_table_refuses_new_task() {
        let mut t = Table::new();
        for i in 0..MAX_ARMED {
            t.arm(TaskId(i as u32), 1).unwrap();
        }
        assert_eq!(t.arm(TaskId(999), 1), Err(ArmError::Full));
        // Replace existing still works.
        t.arm(TaskId(0), 2).unwrap();
    }

    #[test]
    fn multiple_expire_same_poll() {
        let mut t = Table::new();
        t.arm(TaskId(1), 3).unwrap();
        t.arm(TaskId(2), 4).unwrap();
        t.arm(TaskId(3), 100).unwrap();
        let mut out = [TaskId(0); MAX_ARMED];
        let n = t.poll(4, &mut out);
        assert_eq!(n, 2);
        let mut ids = [out[0].0, out[1].0];
        ids.sort();
        assert_eq!(ids, [1, 2]);
    }
}
