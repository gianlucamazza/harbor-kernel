//! Board bind for the BCM2711 power-management block.

use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::memmap;
use crate::drivers::pm::{ResetStatus, read_status};

/// Why this board last reset.
///
/// # Safety
///
/// Reads one register in the PM window. Sound while only core 0 runs and no
/// other subsystem claims the block — nothing else does, because this is the
/// only code that touches it.
pub unsafe fn reset_status() -> ResetStatus {
    // SAFETY: `PM_BASE` is the PM window on the BCM2711 low peripheral map,
    // inside the `peripherals` region the kernel map already covers.
    unsafe { read_status(Mmio::new(memmap::PM_BASE)) }
}
