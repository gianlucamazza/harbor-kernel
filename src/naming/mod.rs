//! EL1 name registry façade (ADR-0035 / P5).
//!
//! Pure table in [`kernel_core::naming`]; this module owns the global and
//! serialises access with [`Mutex`] (ADR-0077, ADR-0091 — dual-current cores may
//! resolve/bind concurrently).

use kernel_core::cap::CapId;
use kernel_core::naming::Table;

use crate::sync::Mutex;

pub use kernel_core::naming::{BindError, ResolveError};

static NAMES: Mutex<Table> = Mutex::new(Table::new());

/// Bind `name` to `cap` (replace if the name exists).
pub fn bind(name: &[u8], cap: CapId) -> Result<(), BindError> {
    NAMES.with(|t| t.bind(name, cap))
}

/// Resolve `name` to a CapId (may be stale after channel revoke).
pub fn resolve(name: &[u8]) -> Result<CapId, ResolveError> {
    NAMES.with(|t| t.resolve(name))
}

/// Remove a binding.
pub fn unbind(name: &[u8]) -> Result<(), ResolveError> {
    NAMES.with(|t| t.unbind(name))
}
