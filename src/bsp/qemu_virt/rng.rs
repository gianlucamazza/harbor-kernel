//! QEMU `virt` has no BCM RNG200 device.

use crate::drivers::rng200::{Rng200, RngError};

/// Keep the RNG vocabulary vacant on this board.
///
/// # Safety
/// Kept unsafe for the board API symmetry; no MMIO is accessed.
pub unsafe fn init() -> Result<Rng200, RngError> {
    Err(RngError::NotPresent)
}
