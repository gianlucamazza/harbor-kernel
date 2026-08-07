//! IRQ notification capability façade (ADR-0030).
//!
//! Pure table lives in [`kernel_core::irqcap`]; this module owns the global and
//! the interrupt mask. Mint is bootstrap-only for the first slice.

use kernel_core::cap::CapId;
use kernel_core::irqcap::{LookupError, MintError, Table};

use crate::arch::cpu;
use crate::sync::SyncCell;

static IRQ_CAPS: SyncCell<Table> = SyncCell::new(Table::new());

/// Mint a notification for `cookie`. Bootstrap only in this slice.
pub fn mint(cookie: u32) -> Result<CapId, MintError> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let table = unsafe { &mut *IRQ_CAPS.get() };
        table.mint(cookie)
    })
}

/// Resolve a CapId to its IRQ cookie, or `BadCap`.
pub fn lookup(cap: CapId) -> Result<u32, LookupError> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let table = unsafe { &*IRQ_CAPS.get() };
        table.lookup(cap)
    })
}
