//! BCM power-management block — read-only access to the reset-cause latch.
//!
//! Board-agnostic: the caller supplies the MMIO base. Bit decode lives in
//! [`kernel_core::reset`], where it is host-tested; this module owns only the
//! register offset and the read.
//!
//! **Read-only, deliberately.** The same block holds `PM_RSTC` and `PM_WDOG`,
//! which reboot the board and arm the watchdog. Writes to any of them require
//! the `0x5a` password in the top byte, and a write with the wrong value is a
//! reset rather than an error. Nothing here needs to write, so nothing here
//! can: there is no `write` function to call by mistake.
//!
//! Reference: Linux `drivers/watchdog/bcm2835_wdt.c`. Broadcom's public
//! datasheet marks the PM block "not for publication", so that driver is the
//! only written-down source for these offsets.

use kernel_core::reset::{ResetCause, cause, partition};

use crate::arch::mmio::Mmio;

/// `PM_RSTS` — reset status, latched by the hardware across a reset.
const RSTS: usize = 0x20;

/// What the last reset was, and which partition the firmware was pointed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResetStatus {
    pub cause: ResetCause,
    pub partition: u32,
    /// The raw register, because a decode that loses the original cannot be
    /// argued with later. The value in a transcript is the evidence; the
    /// decode is this kernel's reading of it.
    pub raw: u32,
}

/// Read the reset-cause latch.
///
/// No probe wrapper, unlike [`crate::drivers::rng200`]: this register is inside
/// the peripheral window the kernel maps for itself, so the access cannot fault
/// for want of a mapping.
///
/// QEMU `raspi4b` does model the block and reports `PM_RSTS=0x00001000`, a
/// power-on reset. That was worth checking rather than assuming: this comment
/// first claimed the emulator returned zero, by analogy with RNG200, and the
/// first boot said otherwise. A block that genuinely reads zero decodes to
/// [`ResetCause::None`] rather than to a power-on, which is what keeps an
/// absent register from manufacturing an answer.
///
/// # Safety
///
/// `regs` must be the PM block's MMIO window, mapped Device-nGnRnE.
pub unsafe fn read_status(regs: Mmio) -> ResetStatus {
    let raw = regs.read32(RSTS);
    ResetStatus {
        cause: cause(raw),
        partition: partition(raw),
        raw,
    }
}
