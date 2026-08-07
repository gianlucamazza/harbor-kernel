//! EL1 keyed blob store façade (ADR-0036 / P2).
//!
//! Pure table in [`kernel_core::storage`]; this module owns the global and the
//! interrupt mask. Put/get are trusted creator paths for this slice.

use kernel_core::storage::Table;

use crate::arch::cpu;
use crate::sync::SyncCell;

pub use kernel_core::storage::{GetError, PutError};

static STORE: SyncCell<Table> = SyncCell::new(Table::new());

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let table = unsafe { &mut *STORE.get() };
        f(table)
    })
}

/// Insert or replace a blob under `key`.
pub fn put(key: &[u8], payload: &[u8]) -> Result<(), PutError> {
    with_table(|t| t.put(key, payload))
}

/// Copy the blob for `key` into `out`; returns bytes written.
pub fn get(key: &[u8], out: &mut [u8]) -> Result<usize, GetError> {
    with_table(|t| t.get(key, out))
}

/// Remove a blob.
pub fn delete(key: &[u8]) -> Result<(), GetError> {
    with_table(|t| t.delete(key))
}
