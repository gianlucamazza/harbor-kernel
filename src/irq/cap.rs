//! IRQ notification capability façade (ADR-0030).
//!
//! Pure table lives in [`kernel_core::irqcap`]; this module owns the global and
//! serialises access with [`IrqSpinLock`] (ADR-0077 / F-R1-P1). Mint is
//! bootstrap-only for the first slice; lookup is the EL0 wait path.

use kernel_core::cap::CapId;
use kernel_core::irqcap::{LookupError, MintError, Table};

use crate::sync::{IrqSpinLock, SyncCell};

static IRQ_CAPS: SyncCell<Table> = SyncCell::new(Table::new());
static IRQ_CAPS_LOCK: IrqSpinLock = IrqSpinLock::new();

fn with_table<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    IRQ_CAPS_LOCK.with(|| {
        // SAFETY: exclusivity from IRQ_CAPS_LOCK.
        f(unsafe { &mut *IRQ_CAPS.get() })
    })
}

/// Mint a notification for `cookie`. Bootstrap only in this slice.
pub fn mint(cookie: u32) -> Result<CapId, MintError> {
    with_table(|table| table.mint(cookie))
}

/// Resolve a CapId to its IRQ cookie, or `BadCap`.
pub fn lookup(cap: CapId) -> Result<u32, LookupError> {
    with_table(|table| table.lookup(cap))
}
