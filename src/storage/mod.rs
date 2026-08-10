//! EL1 keyed blob store façade (ADR-0036 / P2).
//!
//! Pure table in [`kernel_core::storage`]; this module owns the global and
//! serialises access with [`IrqSpinLock`] (ADR-0077).

use kernel_core::storage::Table;

use crate::sync::{IrqSpinLock, SyncCell};

pub use kernel_core::storage::{GetError, PutError};

static STORE: SyncCell<Table> = SyncCell::new(Table::new());
static STORE_LOCK: IrqSpinLock = IrqSpinLock::new();

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    STORE_LOCK.with(|| {
        // SAFETY: exclusivity from STORE_LOCK (IRQ mask + spin).
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
