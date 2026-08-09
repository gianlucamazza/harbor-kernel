//! Per-task capability slots (ADR-0063) — pure, host-tested.
//!
//! The whole of what an EL0 agent may name is a slot index into its own row
//! here (ADR-0017 §2). This table owns the storage and every slot *decision*:
//! resolution, install occupancy, the transfer arithmetic with the ADR-0055
//! endpoint-band filter, and the drain on exit. The kernel keeps what a pure
//! table cannot know — who is asking, whether the destination task is live
//! (epoch-checked, ADR-0062), and the interrupt mask around the operation.
//!
//! Rows are named by **task slot index**, already validated by the caller
//! against [`crate::tasks::Tasks`]; a `TaskId` never reaches this module. An
//! out-of-range row refuses like an out-of-range slot rather than panicking,
//! so a corrupted index is a refusal in a syscall and not an abort.

use crate::cap::{self, CapClass, CapId, SlotError};

/// Why [`Table::install`] refused. One class on purpose: slot out of range
/// and slot occupied carry the same ABI detail (ADR-0061 `BadSlot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    BadSlot,
}

/// Why [`Table::transfer`] refused. `BadToTask` is deliberately absent:
/// destination liveness is not slot arithmetic, and stays a kernel refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferError {
    /// Source slot empty or out of range.
    BadFromSlot,
    /// Destination slot already holds a cap.
    ToSlotFull,
    /// Destination slot index out of range.
    ToSlotOob,
    /// Moved object is not an IPC endpoint cap (ADR-0055).
    Untransferable,
}

/// Pure slot table: `TASKS` rows of `SLOTS` capability slots.
#[derive(Clone)]
pub struct Table<const TASKS: usize, const SLOTS: usize> {
    slots: [[Option<CapId>; SLOTS]; TASKS],
}

impl<const TASKS: usize, const SLOTS: usize> Table<TASKS, SLOTS> {
    pub const fn new() -> Self {
        Self {
            slots: [[None; SLOTS]; TASKS],
        }
    }

    /// Resolve `slot` in `task`'s row.
    ///
    /// The bound comes from [`cap::from_slot`] against the row itself
    /// (ADR-0017 §2); an out-of-range `task` answers as a row with no slots.
    pub fn get(&self, task: usize, slot: usize) -> Result<CapId, SlotError> {
        match self.slots.get(task) {
            Some(row) => cap::from_slot(row, slot),
            None => Err(SlotError::OutOfRange { slot, slots: 0 }),
        }
    }

    /// Whether `task`'s row holds `cap` in any slot.
    pub fn holds(&self, task: usize, cap: CapId) -> bool {
        self.slots
            .get(task)
            .is_some_and(|row| row.contains(&Some(cap)))
    }

    /// A spawn's initial row, holes included (ADR-0017 §2 — a gap is not
    /// padding). Slots past `caps.len()` are cleared.
    pub fn seed(&mut self, task: usize, caps: &[Option<CapId>]) {
        let Some(row) = self.slots.get_mut(task) else {
            return;
        };
        for (i, slot) in row.iter_mut().enumerate() {
            *slot = caps.get(i).copied().flatten();
        }
    }

    /// Install `cap` into `task`'s empty `slot`.
    pub fn install(&mut self, task: usize, slot: usize, cap: CapId) -> Result<(), InstallError> {
        let row = match self.slots.get_mut(task) {
            Some(row) if slot < SLOTS => row,
            _ => return Err(InstallError::BadSlot),
        };
        if row[slot].is_some() {
            return Err(InstallError::BadSlot);
        }
        row[slot] = Some(cap);
        Ok(())
    }

    /// The bounds half of [`Self::transfer`], callable on its own: the ABI
    /// refuses out-of-range slots *before* the kernel's destination-liveness
    /// check (ADR-0061 detail order), so the kernel asks this first, then
    /// liveness, then the full transfer — one owner, called twice.
    pub const fn transfer_bounds(from_slot: usize, to_slot: usize) -> Result<(), TransferError> {
        if from_slot >= SLOTS {
            return Err(TransferError::BadFromSlot);
        }
        if to_slot >= SLOTS {
            return Err(TransferError::ToSlotOob);
        }
        Ok(())
    }

    /// Move `from_task`'s cap at `from_slot` into `to_task`'s empty `to_slot`.
    ///
    /// The refusal order is the ABI's (ADR-0061 details 3/6/7/5): source
    /// bounds, destination bounds, the same-slot no-op, source resolution,
    /// the ADR-0055 endpoint-band filter ([`CapId::classify`] is the one
    /// decoder — ADR-0059), destination occupancy. Caller has already
    /// validated both rows name live tasks.
    pub fn transfer(
        &mut self,
        from_task: usize,
        from_slot: usize,
        to_task: usize,
        to_slot: usize,
    ) -> Result<(), TransferError> {
        Self::transfer_bounds(from_slot, to_slot)?;
        if from_task == to_task && from_slot == to_slot {
            return Ok(());
        }
        let cap = match self.slots[from_task][from_slot] {
            Some(c) => c,
            None => return Err(TransferError::BadFromSlot),
        };
        // ADR-0055: only IPC endpoint caps move. A task-cap as the moved
        // object is delegation — a declared non-goal — and an IRQ cap would
        // hand off ADR-0030's single-armer identity.
        if !matches!(cap.classify(), CapClass::Endpoint(_)) {
            return Err(TransferError::Untransferable);
        }
        if self.slots[to_task][to_slot].is_some() {
            return Err(TransferError::ToSlotFull);
        }
        self.slots[from_task][from_slot] = None;
        self.slots[to_task][to_slot] = Some(cap);
        Ok(())
    }

    /// Take and clear `task`'s row (on exit), for hold release.
    pub fn drain(&mut self, task: usize) -> [Option<CapId>; SLOTS] {
        let Some(row) = self.slots.get_mut(task) else {
            return [None; SLOTS];
        };
        core::mem::replace(row, [None; SLOTS])
    }
}

impl<const TASKS: usize, const SLOTS: usize> Default for Table<TASKS, SLOTS> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cap::{IRQ_BAND, TASK_BAND};

    type T = Table<3, 4>;

    fn ep(n: u16) -> CapId {
        CapId::new(n, 1)
    }

    #[test]
    fn get_resolves_bounds_holes_and_rows() {
        let mut t = T::new();
        t.seed(1, &[Some(ep(7)), None]);
        assert_eq!(t.get(1, 0), Ok(ep(7)));
        assert_eq!(t.get(1, 1), Err(SlotError::Empty { slot: 1 }));
        assert_eq!(
            t.get(1, 4),
            Err(SlotError::OutOfRange { slot: 4, slots: 4 })
        );
        assert_eq!(
            t.get(9, 0),
            Err(SlotError::OutOfRange { slot: 0, slots: 0 })
        );
    }

    #[test]
    fn seed_clears_slots_past_the_given_prefix() {
        let mut t = T::new();
        t.seed(0, &[Some(ep(1)), Some(ep(2)), Some(ep(3)), Some(ep(4))]);
        t.seed(0, &[Some(ep(9))]);
        assert_eq!(t.get(0, 0), Ok(ep(9)));
        for slot in 1..4 {
            assert_eq!(t.get(0, slot), Err(SlotError::Empty { slot }));
        }
    }

    #[test]
    fn holds_answers_both_ways() {
        let mut t = T::new();
        t.seed(2, &[None, Some(ep(5))]);
        assert!(t.holds(2, ep(5)));
        assert!(!t.holds(2, ep(6)));
        assert!(!t.holds(9, ep(5)), "out-of-range row holds nothing");
    }

    #[test]
    fn install_refuses_oob_occupied_and_bad_row() {
        let mut t = T::new();
        assert_eq!(t.install(0, 4, ep(1)), Err(InstallError::BadSlot));
        assert_eq!(t.install(9, 0, ep(1)), Err(InstallError::BadSlot));
        assert_eq!(t.install(0, 2, ep(1)), Ok(()));
        assert_eq!(t.install(0, 2, ep(2)), Err(InstallError::BadSlot));
        assert_eq!(t.get(0, 2), Ok(ep(1)), "the refusal did not overwrite");
    }

    #[test]
    fn transfer_moves_an_endpoint_cap() {
        let mut t = T::new();
        t.seed(0, &[Some(ep(3))]);
        assert_eq!(t.transfer(0, 0, 1, 2), Ok(()));
        assert_eq!(
            t.get(0, 0),
            Err(SlotError::Empty { slot: 0 }),
            "donor emptied"
        );
        assert_eq!(t.get(1, 2), Ok(ep(3)));
    }

    #[test]
    fn transfer_refusal_order_is_the_abi_order() {
        let mut t = T::new();
        t.seed(0, &[Some(ep(3)), Some(ep(4))]);
        t.seed(1, &[Some(ep(8))]);
        assert_eq!(t.transfer(0, 4, 1, 0), Err(TransferError::BadFromSlot));
        assert_eq!(t.transfer(0, 0, 1, 4), Err(TransferError::ToSlotOob));
        assert_eq!(t.transfer(0, 2, 1, 1), Err(TransferError::BadFromSlot));
        assert_eq!(t.transfer(0, 0, 1, 0), Err(TransferError::ToSlotFull));
        // Out-of-range for BOTH from-slot and to-slot: the source refusal
        // wins, which is what ADR-0061's detail codes encode.
        assert_eq!(t.transfer(0, 4, 1, 4), Err(TransferError::BadFromSlot));
    }

    #[test]
    fn same_task_same_slot_is_a_no_op_even_when_empty() {
        let mut t = T::new();
        assert_eq!(t.transfer(0, 3, 0, 3), Ok(()), "before any seed");
        t.seed(0, &[Some(ep(3))]);
        assert_eq!(t.transfer(0, 0, 0, 0), Ok(()));
        assert_eq!(t.get(0, 0), Ok(ep(3)), "still there");
    }

    #[test]
    fn same_task_different_slot_really_moves() {
        let mut t = T::new();
        t.seed(0, &[Some(ep(3))]);
        assert_eq!(t.transfer(0, 0, 0, 1), Ok(()));
        assert_eq!(t.get(0, 0), Err(SlotError::Empty { slot: 0 }));
        assert_eq!(t.get(0, 1), Ok(ep(3)));
    }

    #[test]
    fn non_endpoint_bands_are_untransferable() {
        // ADR-0055 via ADR-0059's one decoder: a task-cap, an IRQ cap and the
        // invalid both-bands quadrant all refuse — before the destination is
        // even looked at (the occupied dest must not shadow the class).
        let mut t = T::new();
        for bad in [TASK_BAND | 1, IRQ_BAND | 1, TASK_BAND | IRQ_BAND | 1] {
            t.seed(0, &[Some(CapId::new(bad, 1))]);
            t.seed(1, &[Some(ep(8))]);
            assert_eq!(t.transfer(0, 0, 1, 0), Err(TransferError::Untransferable));
            assert_eq!(t.transfer(0, 0, 1, 1), Err(TransferError::Untransferable));
        }
    }

    #[test]
    fn drain_takes_the_row_once_and_leaves_others() {
        let mut t = T::new();
        t.seed(0, &[Some(ep(1)), None, Some(ep(2))]);
        t.seed(1, &[Some(ep(9))]);
        let row = t.drain(0);
        assert_eq!(row, [Some(ep(1)), None, Some(ep(2)), None]);
        assert_eq!(t.drain(0), [None; 4], "already cleared");
        assert_eq!(t.get(1, 0), Ok(ep(9)), "other rows untouched");
        assert_eq!(t.drain(9), [None; 4], "out-of-range row drains empty");
    }
}
