//! EL1 keyed blob store façade (ADR-0036 / P2).
//!
//! Pure table in [`kernel_core::storage`]; this module owns the global and
//! serialises access with [`Mutex`] (ADR-0077, ADR-0091).

use kernel_core::storage::Table;

use crate::sync::Mutex;

pub use kernel_core::storage::{GetError, PutError};

static STORE: Mutex<Table> = Mutex::new(Table::new());

/// Insert or replace a blob under `key`.
pub fn put(key: &[u8], payload: &[u8]) -> Result<(), PutError> {
    STORE.with(|t| t.put(key, payload))
}

/// Copy the blob for `key` into `out`; returns bytes written.
pub fn get(key: &[u8], out: &mut [u8]) -> Result<usize, GetError> {
    STORE.with(|t| t.get(key, out))
}

/// Remove a blob.
pub fn delete(key: &[u8]) -> Result<(), GetError> {
    STORE.with(|t| t.delete(key))
}
