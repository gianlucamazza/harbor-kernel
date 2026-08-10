//! EL1 name registry façade (ADR-0035 / P5).
//!
//! Pure table in [`kernel_core::naming`]; this module owns the global and
//! serialises access with [`IrqSpinLock`] (ADR-0077 — dual-current cores may
//! resolve/bind concurrently).

use kernel_core::cap::CapId;
use kernel_core::naming::Table;

use crate::sync::{IrqSpinLock, SyncCell};

pub use kernel_core::naming::{BindError, ResolveError};

static NAMES: SyncCell<Table> = SyncCell::new(Table::new());
static NAMES_LOCK: IrqSpinLock = IrqSpinLock::new();

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    NAMES_LOCK.with(|| {
        // SAFETY: exclusivity from NAMES_LOCK (IRQ mask + spin).
        let table = unsafe { &mut *NAMES.get() };
        f(table)
    })
}

/// Bind `name` to `cap` (replace if the name exists).
pub fn bind(name: &[u8], cap: CapId) -> Result<(), BindError> {
    with_table(|t| t.bind(name, cap))
}

/// Resolve `name` to a CapId (may be stale after channel revoke).
pub fn resolve(name: &[u8]) -> Result<CapId, ResolveError> {
    with_table(|t| t.resolve(name))
}

/// Remove a binding.
pub fn unbind(name: &[u8]) -> Result<(), ResolveError> {
    with_table(|t| t.unbind(name))
}
