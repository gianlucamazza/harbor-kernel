//! BCM2835/BCM2711 SPI0 master — polled, board-agnostic.
//!
//! The caller supplies the MMIO base and a pre-computed `CDIV` encoding from
//! [`kernel_core::spi::clock_divisor`]. Chip-select is **not** driven here:
//! use [`super::ExclusiveDevice`] with a GPIO CS so multiple slaves can share
//! the bus (ADR-0009). Pinmux the CE pins as GPIO outputs, not as SPI ALT0.
//!
//! Reference: BCM2835 ARM Peripherals, §10 SPI.

use kernel_core::poll;

use crate::arch::mmio::Mmio;
use crate::drivers::spi::SpiBus;

// Register offsets (bytes).
const CS: usize = 0x00;
const FIFO: usize = 0x04;
const CLK: usize = 0x08;

// CS bits.
const CS_TA: u32 = 1 << 7;
const CS_CLEAR: u32 = 0b11 << 4;
const CS_DONE: u32 = 1 << 16;
const CS_RXD: u32 = 1 << 17;
const CS_TXD: u32 = 1 << 18;

/// Upper bound on spins waiting for TX FIFO space or DONE.
///
/// Sized like the PL011 TX wait: a wedged SPI block must not hang the panic
/// path forever. At a few MHz bit clock a byte leaves in microseconds; this
/// budget is many milliseconds of CPU time on a 1.5 GHz core.
const SPIN_LIMIT: u32 = 10_000_000;

/// Why a transfer could not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BcmSpiError {
    /// `read` and `write` slices differed in length.
    LengthMismatch,
    /// TX ready, RX data, or DONE never asserted within the spin budget.
    Timeout,
}

/// Polled SPI0 controller in mode 0 (CPOL=0, CPHA=0), 8-bit words.
pub struct BcmSpi {
    regs: Mmio,
}

impl BcmSpi {
    /// Bind SPI0 at `mmio` and program `cdiv` (encoding from `kernel_core::spi`).
    ///
    /// Leaves transfer inactive (`TA=0`) with FIFOs cleared. Safe to call again
    /// to re-rate the bus.
    ///
    /// # Safety
    ///
    /// `mmio` must address the SPI0 register block exclusive to this driver
    /// for the duration of use. CE pins must not be claimed as SPI ALT functions
    /// if software CS is used.
    pub unsafe fn init(mmio: Mmio, cdiv: u32) -> Self {
        let spi = Self { regs: mmio };
        spi.configure(cdiv);
        spi
    }

    fn configure(&self, cdiv: u32) {
        // Idle: TA clear, FIFOs cleared, mode 0 (CPOL/CPHA zero after reset write).
        self.regs.write32(CS, CS_CLEAR);
        self.regs.write32(CLK, cdiv);
        // Keep TA low; CS chip-select field unused when CE pins are GPIO.
        self.regs.write32(CS, 0);
    }

    fn wait_cs(&self, mask: u32) -> Result<(), BcmSpiError> {
        let regs = self.regs;
        if poll::until(SPIN_LIMIT, || regs.read32(CS) & mask != 0) {
            Ok(())
        } else {
            Err(BcmSpiError::Timeout)
        }
    }

    /// One full-duplex byte: wait TX slot, write, wait RX byte, read.
    fn transfer_byte(&self, out: u8) -> Result<u8, BcmSpiError> {
        self.wait_cs(CS_TXD)?;
        self.regs.write32(FIFO, u32::from(out));
        self.wait_cs(CS_RXD)?;
        Ok(self.regs.read32(FIFO) as u8)
    }

    fn start(&self) {
        // Clear FIFOs, then raise TA. Mode 0 bits remain zero.
        self.regs.write32(CS, CS_CLEAR);
        self.regs.write32(CS, CS_TA);
    }

    fn stop(&self) -> Result<(), BcmSpiError> {
        // Drain any residual RX before dropping TA.
        while self.regs.read32(CS) & CS_RXD != 0 {
            let _ = self.regs.read32(FIFO);
        }
        self.wait_cs(CS_DONE)?;
        self.regs.write32(CS, 0);
        Ok(())
    }
}

impl SpiBus for BcmSpi {
    type Error = BcmSpiError;

    fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        self.start();
        for &b in words {
            let _ = self.transfer_byte(b)?;
        }
        self.stop()
    }

    fn transfer(&mut self, read: &mut [u8], words: &[u8]) -> Result<(), Self::Error> {
        if read.len() != words.len() {
            return Err(BcmSpiError::LengthMismatch);
        }
        self.start();
        for (slot, &b) in read.iter_mut().zip(words.iter()) {
            *slot = self.transfer_byte(b)?;
        }
        self.stop()
    }

    fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        self.start();
        for slot in words.iter_mut() {
            *slot = self.transfer_byte(*slot)?;
        }
        self.stop()
    }
}
