//! Kernel owner of the task-cap table (ADR-0054 / K3).
//!
//! Pure arithmetic lives in [`kernel_core::taskcap`]. This module holds the
//! single table and serialises access with IRQ masking.

use kernel_core::cap::CapId;
use kernel_core::runqueue::TaskId;
use kernel_core::taskcap::Table;

use crate::arch::cpu;
use crate::sync::SyncCell;

pub use kernel_core::taskcap::{LookupError, MintError};

static TABLE: SyncCell<Table> = SyncCell::new(Table::new());

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let table = unsafe { &mut *TABLE.get() };
        f(table)
    })
}

/// Mint a task-cap naming `id` (trusted EL1 / creator path).
pub fn mint(id: TaskId) -> Result<CapId, MintError> {
    with_table(|t| t.mint(id.0))
}

/// Resolve a held CapId to a task id if it is a live task-cap.
pub fn lookup(cap: CapId) -> Result<TaskId, LookupError> {
    with_table(|t| t.lookup(cap).map(TaskId))
}

/// Invalidate every task-cap naming `id` (call on task exit).
pub fn revoke_task(id: TaskId) -> u32 {
    with_table(|t| t.revoke_task(id.0))
}
