//! Board bind for the SD card behind an SDHCI host (ADR-0066).
//!
//! BCM2711 has two SDHCI-class hosts the card slot can be routed to: EMMC2
//! (the silicon default for the SD slot) and the legacy Arasan block. QEMU
//! `raspi4b` wires the `-drive if=sd` card into the Arasan, silicon into
//! EMMC2 — so the bind tries EMMC2 first and falls back, and reports which
//! one answered rather than pretending there is only one.

use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::memmap;
use crate::drivers::sdhci::{SdError, Sdhci};

/// Bring up the SD card and return a ready handle plus the name of the
/// host that answered (`"emmc2"` or `"arasan"`).
///
/// Failure is not fatal for the board: the caller logs the degraded line
/// and continues without media persistence. When both hosts refuse, the
/// EMMC2 error is reported — it is the silicon-meaningful one.
///
/// # Safety
///
/// Exclusive access to both SDHCI MMIO windows. Holds while core 0 is the
/// only core driving devices (core 1 parks with IRQs masked, ADR-0070) and no
/// other subsystem claims either block.
pub unsafe fn init() -> Result<(Sdhci, &'static str), SdError> {
    // SAFETY: both bases are SDHCI windows on the BCM2711 low peripheral
    // map, covered by the kernel's Device mapping.
    match unsafe { Sdhci::init(Mmio::new(memmap::EMMC2_BASE)) } {
        Ok(sd) => Ok((sd, "emmc2")),
        // SAFETY: as above — the legacy Arasan window, same map, same
        // exclusivity; tried only after EMMC2 refused, so at most one host
        // ever ends up owned.
        Err(emmc2_err) => match unsafe { Sdhci::init(Mmio::new(memmap::SDHCI_LEGACY_BASE)) } {
            Ok(sd) => Ok((sd, "arasan")),
            Err(_) => Err(emmc2_err),
        },
    }
}
