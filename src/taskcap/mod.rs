//! Kernel owner of the task-cap table (ADR-0054 / K3).
//!
//! Pure arithmetic lives in [`kernel_core::taskcap`]. This module holds the
//! single table and serialises access with [`Mutex`] (ADR-0077, ADR-0091).

use kernel_core::cap::CapId;
use kernel_core::runqueue::TaskId;
use kernel_core::taskcap::Table;

use crate::sync::Mutex;

pub use kernel_core::taskcap::{LookupError, MintError};

static TABLE: Mutex<Table> = Mutex::new(Table::new());

/// Mint a task-cap naming `id` (trusted EL1 / creator path).
pub fn mint(id: TaskId) -> Result<CapId, MintError> {
    TABLE.with(|t| t.mint(id.to_raw()))
}

/// Resolve a held CapId to a task id if it is a live task-cap.
pub fn lookup(cap: CapId) -> Result<TaskId, LookupError> {
    TABLE.with(|t| t.lookup(cap).map(TaskId::from_raw))
}

/// Invalidate every task-cap naming `id` (call on task exit).
pub fn revoke_task(id: TaskId) -> u32 {
    TABLE.with(|t| t.revoke_task(id.to_raw()))
}
