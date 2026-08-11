//! Task capabilities (ADR-0053 / ADR-0054 / K3) — pure, host-tested.
//!
//! A task-cap names a live task for **cap install into that task's empty slots**
//! (peer transfer). Indices sit in a dedicated band so they never collide with
//! IPC endpoint indices or IRQ-cap indices (`0x8000`).

use crate::cap::CapId;
use crate::irqcap;

/// Concurrent task-cap objects.
///
/// Deliberately below `MAX_TASKS` (ADR-0057 §2): a pressure bound, and entries
/// are freed only by [`Table::revoke_task`] — there is no per-cap free.
///
/// The gap has widened as `MAX_TASKS` grew (40 → 54) and that is not drift:
/// this bounds how many task-caps may be *live at once*, which is a property
/// of how many supervisors hold references, not of how many tasks exist. It
/// moves when minting starts refusing under a real composition, not when the
/// task table does.
pub const MAX_TASK_CAPS: usize = 32;

/// Index band: `INDEX_BASE | local` (local < MAX_TASK_CAPS).
///
/// Restates [`crate::cap::TASK_BAND`] — the class encoding is ADR-0059's, and
/// [`crate::cap::CapId::classify`] is the one decoder.
pub const INDEX_BASE: u16 = crate::cap::TASK_BAND;

// The bands must never share a bit: a forged id carrying both would otherwise
// be decodable by two tables. Checked here so a moved band fails the build,
// not a review.
const _: () = assert!(
    INDEX_BASE & irqcap::INDEX_BASE == 0,
    "task-cap and IRQ-cap bands must be disjoint"
);

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
    /// Task id bits (`TaskId::to_raw`), epoch included (ADR-0062).
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
    ///
    /// Trusted EL1 path: the pure table cannot check liveness, so the caller
    /// guarantees `task_id` names a live task (ADR-0057 §2). The u16
    /// generation wraps after 65 535 mint cycles on one local index, at which
    /// point a stale handle re-validates — a decided bound (ADR-0057 §3),
    /// unreachable from the current boot.
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
        // ADR-0059: the class decode is total; anything that is not a
        // task-cap — endpoint, IRQ, or the invalid both-bands quadrant —
        // refuses here.
        let local = match cap.classify() {
            crate::cap::CapClass::Task(local) => local as usize,
            _ => return Err(LookupError::BadCap),
        };
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

    #[test]
    fn endpoint_shaped_id_never_hits_a_live_entry() {
        // The band decode, not just the empty-table refusal: local index and
        // generation MATCH a live entry, and the id must still refuse because
        // it does not carry the band. cargo-mutants proved the older test
        // (empty table) could not tell the decode from the entry check.
        let mut t = Table::new();
        let cap = t.mint(3).unwrap();
        let low = CapId::new(cap.index() & !INDEX_BASE, cap.generation());
        assert_eq!(t.lookup(low), Err(LookupError::BadCap));
        let both_bands = CapId::new(cap.index() | irqcap::INDEX_BASE, cap.generation());
        assert_eq!(t.lookup(both_bands), Err(LookupError::BadCap));
    }

    #[test]
    fn revoke_task_leaves_other_tasks_caps_live() {
        let mut t = Table::new();
        let five = t.mint(5).unwrap();
        let seven = t.mint(7).unwrap();
        assert_eq!(t.revoke_task(5), 1);
        assert_eq!(t.lookup(five), Err(LookupError::BadCap));
        assert_eq!(t.lookup(seven), Ok(7));
    }

    #[cfg_attr(
        miri,
        ignore = "65 k mint/revoke cycles interpreted take tens of minutes; the loop is pure arithmetic with no unsafe — this crate's only unsafe is ring.rs, which the unit tests already put under Miri"
    )]
    #[test]
    fn generation_wrap_revalidates_after_65535_cycles() {
        // ADR-0057 §3: the u16 generation wraps (skipping 0) after 65 535 mint
        // cycles on one local index, and the original stale handle validates
        // again. This test *encodes the decided bound* rather than pretending
        // the guarantee is unbounded.
        let mut t = Table::new();
        let first = t.mint(7).unwrap();
        t.revoke_task(7);
        assert_eq!(t.lookup(first), Err(LookupError::BadCap));
        for _ in 0..65534 {
            let c = t.mint(7).unwrap();
            assert_eq!(c.index(), first.index());
            t.revoke_task(7);
        }
        let wrapped = t.mint(7).unwrap();
        assert_eq!(wrapped.generation(), first.generation());
        assert_eq!(t.lookup(first), Ok(7));
    }
}
