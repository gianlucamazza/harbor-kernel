//! EL1 name registry façade (ADR-0035 / P5).
//!
//! Pure table in [`kernel_core::naming`]; this module owns the global and the
//! interrupt mask. Bind/resolve are trusted creator paths for this slice.

use kernel_core::cap::CapId;
use kernel_core::naming::Table;

use crate::arch::cpu;
use crate::sync::SyncCell;

pub use kernel_core::naming::{BindError, ResolveError};

static NAMES: SyncCell<Table> = SyncCell::new(Table::new());

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
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
