//! Task capabilities (ADR-0053 / ADR-0054 / K3) — pure, host-tested.
//!
//! A task-cap names a live task for **cap install into that task's empty slots**
//! (peer transfer). Indices sit in a dedicated band so they never collide with
//! IPC endpoint indices or IRQ-cap indices (`0x8000`).

use crate::cap::CapId;

/// Concurrent task-cap objects.
pub const MAX_TASK_CAPS: usize = 32;

/// Index band: `INDEX_BASE | local` (local < MAX_TASK_CAPS).
///
/// Chosen below IRQ (`0x8000`) and above typical endpoint counts.
pub const INDEX_BASE: u16 = 0x4000;

/// Why [`Table::mint`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MintError {
    Full,
}

/// Why [`Table::lookup`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupError {
    BadCap,
}

#[derive(Clone, Copy)]
struct Entry {
    live: bool,
    generation: u16,
    /// Task id bits (`TaskId.0`).
    task: u32,
}

impl Entry {
    const EMPTY: Self = Self {
        live: false,
        generation: 0,
        task: 0,
    };
}

/// Pure table: mint task id → CapId, lookup CapId → task id.
#[derive(Clone)]
pub struct Table {
    entries: [Entry; MAX_TASK_CAPS],
}

impl Table {
    pub const fn new() -> Self {
        Self {
            entries: [Entry::EMPTY; MAX_TASK_CAPS],
        }
    }

    /// Mint a capability naming `task_id`.
    pub fn mint(&mut self, task_id: u32) -> Result<CapId, MintError> {
        for i in 0..MAX_TASK_CAPS {
            if !self.entries[i].live {
                let mut generation = self.entries[i].generation.wrapping_add(1);
                if generation == 0 {
                    generation = 1;
                }
                self.entries[i] = Entry {
                    live: true,
                    generation,
                    task: task_id,
                };
                return Ok(CapId::new(INDEX_BASE | (i as u16), generation));
            }
        }
        Err(MintError::Full)
    }

    /// Resolve a CapId to the named task id if live and generation matches.
    pub fn lookup(&self, cap: CapId) -> Result<u32, LookupError> {
        let idx = cap.index();
        if idx & INDEX_BASE == 0 || idx & 0x8000 != 0 {
            // Not in the task-cap band (or looks like IRQ high bit).
            return Err(LookupError::BadCap);
        }
        let local = (idx & !INDEX_BASE) as usize;
        if local >= MAX_TASK_CAPS {
            return Err(LookupError::BadCap);
        }
        let e = &self.entries[local];
        if !e.live || e.generation != cap.generation() {
            return Err(LookupError::BadCap);
        }
        Ok(e.task)
    }

    /// Invalidate every live entry naming `task_id` (on task exit).
    ///
    /// Returns how many entries were killed.
    pub fn revoke_task(&mut self, task_id: u32) -> u32 {
        let mut n = 0u32;
        for e in &mut self.entries {
            if e.live && e.task == task_id {
                e.live = false;
                // Keep generation so a stale CapId with the old gen fails lookup.
                n = n.saturating_add(1);
            }
        }
        n
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
    fn mint_lookup_round_trip() {
        let mut t = Table::new();
        let cap = t.mint(3).unwrap();
        assert_eq!(t.lookup(cap), Ok(3));
        assert_eq!(cap.index() & INDEX_BASE, INDEX_BASE);
    }

    #[test]
    fn ipc_and_irq_shaped_ids_are_refused() {
        let t = Table::new();
        assert_eq!(t.lookup(CapId::new(0, 1)), Err(LookupError::BadCap));
        assert_eq!(t.lookup(CapId::new(0x8001, 1)), Err(LookupError::BadCap));
    }

    #[test]
    fn revoke_task_stales_handles() {
        let mut t = Table::new();
        let a = t.mint(5).unwrap();
        let b = t.mint(5).unwrap();
        assert_eq!(t.revoke_task(5), 2);
        assert_eq!(t.lookup(a), Err(LookupError::BadCap));
        assert_eq!(t.lookup(b), Err(LookupError::BadCap));
        // Re-mint reuses a free slot with a new generation.
        let c = t.mint(5).unwrap();
        assert_eq!(t.lookup(c), Ok(5));
        assert_ne!(c.generation(), a.generation());
    }

    #[test]
    fn full_table_refuses() {
        let mut t = Table::new();
        for i in 0..MAX_TASK_CAPS {
            t.mint(i as u32).unwrap();
        }
        assert_eq!(t.mint(99), Err(MintError::Full));
    }

    #[test]
    fn stale_generation_is_refused() {
        let mut t = Table::new();
        let cap = t.mint(1).unwrap();
        let stale = CapId::new(cap.index(), cap.generation().wrapping_add(1));
        assert_eq!(t.lookup(stale), Err(LookupError::BadCap));
    }
}
