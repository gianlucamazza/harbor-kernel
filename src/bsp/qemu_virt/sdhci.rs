//! QEMU `virt` has no SDHCI controller in the P3 composition.

use crate::drivers::sdhci::{SdError, Sdhci};

/// Keep durable media explicitly absent on this board.
///
/// # Safety
/// Kept unsafe for the board API symmetry; no MMIO is accessed.
pub unsafe fn init() -> Result<(Sdhci, &'static str), SdError> {
    Err(SdError::NotPresent)
}
