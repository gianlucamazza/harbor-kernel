//! QEMU `virt` has no BCM reset-status block.

use kernel_core::reset::{ResetCause, partition};

use crate::drivers::pm::ResetStatus;

/// Return an explicit unknown/empty reset latch; do not read an absent MMIO
/// address merely to manufacture a QEMU reset claim.
///
/// # Safety
/// Kept unsafe for the board API symmetry; this implementation touches no MMIO.
pub unsafe fn reset_status() -> ResetStatus {
    ResetStatus {
        cause: ResetCause::None,
        partition: partition(0),
        raw: 0,
    }
}
