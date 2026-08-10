//! IRQ notification capability façade (ADR-0030).
//!
//! Pure table lives in [`kernel_core::irqcap`]; this module owns the global and
//! serialises access with [`Mutex`] (ADR-0077, ADR-0091 / F-R1-P1). Mint is
//! bootstrap-only for the first slice; lookup is the EL0 wait path.

use kernel_core::cap::CapId;
use kernel_core::irqcap::{LookupError, MintError, Table};

use crate::sync::Mutex;

static IRQ_CAPS: Mutex<Table> = Mutex::new(Table::new());

/// Mint a notification for `cookie`. Bootstrap only in this slice.
pub fn mint(cookie: u32) -> Result<CapId, MintError> {
    IRQ_CAPS.with(|table| table.mint(cookie))
}

/// Resolve a CapId to its IRQ cookie, or `BadCap`.
pub fn lookup(cap: CapId) -> Result<u32, LookupError> {
    IRQ_CAPS.with(|table| table.lookup(cap))
}
