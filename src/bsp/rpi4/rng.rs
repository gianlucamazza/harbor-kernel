//! Board bind for the BCM2711 RNG200 block.

use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::memmap;
use crate::drivers::rng200::{Rng200, RngError};

/// Bring up the SoC RNG200 and return a ready handle.
///
/// Soft-resets, enables the RBG, and waits for warm-up. Failure is not fatal
/// for the board: the caller should log and continue.
///
/// # Safety
///
/// Exclusive access to the RNG200 MMIO window. Holds while core 0 is the only
/// core driving devices (core 1 parks with IRQs masked, ADR-0070) and no other
/// subsystem claims the block.
pub unsafe fn init() -> Result<Rng200, RngError> {
    // SAFETY: `RNG200_BASE` is the RNG200 window on BCM2711 low peripheral map.
    unsafe { Rng200::init(Mmio::new(memmap::RNG200_BASE)) }
}
