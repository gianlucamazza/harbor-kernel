//! BCM2711 / iProc RNG200 — polled hardware random bit generator.
//!
//! Board-agnostic: the caller supplies the MMIO base. Register encodings live
//! in [`kernel_core::rng`]; this module owns the enable / warm-up / FIFO read
//! sequence and bounded recovery on health failures.
//!
//! Output is raw 32-bit FIFO words from the silicon block. It is **not** a
//! CSPRNG and carries no min-entropy claim — see `docs/hardware.md`.
//!
//! Reference: Linux `drivers/char/hw_random/iproc-rng200.c`, BCM2711 map.

use kernel_core::poll;
use kernel_core::rng::{
    self, INT_STATUS_CLEAR_ALL, SAMPLE_DIVISOR_DEFAULT, SOFT_RESET_BIT, ctrl_enable,
    fifo_available, health_failed, warmup_done,
};

use crate::arch::mmio::Mmio;
use crate::arch::probe;

/// Spins waiting for warm-up bit count or a FIFO word.
///
/// Generous: warm-up is a few dozen bits at ~MHz class rates; a wedged block
/// must not hang boot or panic diagnostics forever.
const WARMUP_SPIN_LIMIT: u32 = 10_000_000;
const FIFO_SPIN_LIMIT: u32 = 10_000_000;

/// Soft-resets allowed per multi-word read (matches Linux read path).
const MAX_RESETS_PER_READ: u32 = 1;

/// Why the RNG could not supply data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RngError {
    /// No device responded at the MMIO window (external abort on probe).
    ///
    /// On real BCM2711 this is unexpected. QEMU `raspi4b` currently has no
    /// RNG200 backend at `0xFE10_4000`, so init reports this instead of panicking.
    NotPresent,
    /// Warm-up or FIFO wait exhausted its spin budget.
    Timeout,
    /// Lockout / NIST fail remained after the reset budget.
    HealthFail,
}

/// Polled RNG200 controller.
pub struct Rng200 {
    regs: Mmio,
}

impl Rng200 {
    /// Soft-reset, clear interrupt status, enable the RBG, wait for warm-up.
    ///
    /// Safe to call again on the same block (full re-init). The first access
    /// is a recoverable probe: a missing bus backend yields
    /// [`RngError::NotPresent`] rather than a fatal data abort.
    ///
    /// # Safety
    ///
    /// `mmio` must address the RNG200 register window exclusive to this driver
    /// for the duration of use, and must be Device-mapped when the block exists.
    pub unsafe fn init(mmio: Mmio) -> Result<Self, RngError> {
        // Presence: one write into the window. Emulators without the block
        // external-abort here; the probe path turns that into NotPresent.
        let ctrl = mmio.base() + rng::RNG_CTRL;
        // SAFETY: Device window; probe recovers external abort at `ctrl`.
        if unsafe { probe::try_write32(ctrl, 0) }.is_err() {
            return Err(RngError::NotPresent);
        }

        let rng = Self { regs: mmio };
        rng.restart()?;
        rng.wait_warmup()?;
        Ok(rng)
    }

    /// Soft-reset RBG + RNG, clear status, re-enable with default sample divisor.
    pub fn restart(&self) -> Result<(), RngError> {
        // Disable before reset (Linux restart path).
        self.regs.write32(rng::RNG_CTRL, 0);

        self.regs.write32(rng::RNG_INT_STATUS, INT_STATUS_CLEAR_ALL);

        // Assert RBG then RNG soft-reset.
        self.regs.write32(rng::RBG_SOFT_RESET, SOFT_RESET_BIT);
        self.regs.write32(rng::RNG_SOFT_RESET, SOFT_RESET_BIT);

        // Deassert RNG then RBG.
        self.regs.write32(rng::RNG_SOFT_RESET, 0);
        self.regs.write32(rng::RBG_SOFT_RESET, 0);

        // Enable with the conventional sample divisor (kernel-core documents it).
        self.regs
            .write32(rng::RNG_CTRL, ctrl_enable(SAMPLE_DIVISOR_DEFAULT));

        // Clear any status raised during enable.
        self.regs.write32(rng::RNG_INT_STATUS, INT_STATUS_CLEAR_ALL);

        if !self.health_ok() {
            return Err(RngError::HealthFail);
        }
        Ok(())
    }

    fn wait_warmup(&self) -> Result<(), RngError> {
        let regs = self.regs;
        if poll::until(WARMUP_SPIN_LIMIT, || {
            warmup_done(regs.read32(rng::RNG_TOTAL_BIT_COUNT))
        }) {
            Ok(())
        } else {
            Err(RngError::Timeout)
        }
    }

    /// True when interrupt status has no lockout / NIST fail bits.
    #[inline]
    pub fn health_ok(&self) -> bool {
        !health_failed(self.regs.read32(rng::RNG_INT_STATUS))
    }

    /// Number of 32-bit words currently in the FIFO.
    #[inline]
    pub fn fifo_count(&self) -> u8 {
        fifo_available(self.regs.read32(rng::RNG_FIFO_COUNT))
    }

    /// One word if the FIFO is non-empty; `Ok(None)` if empty.
    ///
    /// On health failure attempts a single restart, then re-checks.
    pub fn try_word(&self) -> Result<Option<u32>, RngError> {
        self.ensure_healthy()?;
        if self.fifo_count() == 0 {
            return Ok(None);
        }
        Ok(Some(self.regs.read32(rng::RNG_FIFO_DATA)))
    }

    /// Fill `buf` from the FIFO, polling while empty.
    ///
    /// Returns the number of words written. On timeout with a partial fill,
    /// returns `Ok(n)` for `n > 0`, or `Err(Timeout)` if nothing was written.
    /// Health recovery uses at most [`MAX_RESETS_PER_READ`] restarts.
    pub fn read_words(&self, buf: &mut [u32]) -> Result<usize, RngError> {
        let mut filled = 0usize;
        let mut resets = 0u32;

        while filled < buf.len() {
            if !self.health_ok() {
                if resets >= MAX_RESETS_PER_READ {
                    return if filled > 0 {
                        Ok(filled)
                    } else {
                        Err(RngError::HealthFail)
                    };
                }
                self.restart()?;
                self.wait_warmup()?;
                resets += 1;
            }

            if self.fifo_count() > 0 {
                buf[filled] = self.regs.read32(rng::RNG_FIFO_DATA);
                filled += 1;
                continue;
            }

            // Wait for at least one word.
            let regs = self.regs;
            if !poll::until(FIFO_SPIN_LIMIT, || {
                fifo_available(regs.read32(rng::RNG_FIFO_COUNT)) > 0
                    || health_failed(regs.read32(rng::RNG_INT_STATUS))
            }) {
                return if filled > 0 {
                    Ok(filled)
                } else {
                    Err(RngError::Timeout)
                };
            }
            // Loop: health or FIFO will be handled at the top.
        }

        Ok(filled)
    }

    fn ensure_healthy(&self) -> Result<(), RngError> {
        if self.health_ok() {
            return Ok(());
        }
        self.restart()?;
        self.wait_warmup()
    }
}
