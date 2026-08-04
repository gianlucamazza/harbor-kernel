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

/// Upper bound on consecutive spins that make no progress.
///
/// Sized like the PL011 TX wait: a wedged SPI block must not hang the panic
/// path forever. Each spin is an MMIO read of `CS`, which on this SoC costs
/// on the order of a hundred nanoseconds, so this budget is roughly a second
/// of CPU time — long against any real transfer, short against a hang.
const SPIN_LIMIT: u32 = 10_000_000;

/// Bytes each FIFO holds.
///
/// Transfers are cut into chunks this size so a whole chunk can be pushed
/// before any of it is read back: the block stalls when the RX FIFO fills, so
/// writing more than this without draining would deadlock the transfer against
/// itself.
const FIFO_DEPTH: usize = 64;

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

    /// Full-duplex a chunk no larger than [`FIFO_DEPTH`].
    ///
    /// Keeps both FIFOs busy instead of round-tripping each byte: the previous
    /// byte is still on the wire while the next is queued, which is the whole
    /// reason the block has a FIFO. `recv < sent` is what keeps the two indices
    /// honest — a byte can only be read back after its outgoing partner was
    /// written, because SPI produces exactly one input byte per output byte.
    fn transfer_chunk(&self, out: &[u8], inp: &mut [u8]) -> Result<(), BcmSpiError> {
        debug_assert_eq!(out.len(), inp.len());
        debug_assert!(out.len() <= FIFO_DEPTH);

        let regs = self.regs;
        let mut sent = 0usize;
        let mut recv = 0usize;
        let mut idle = 0u32;

        while recv < inp.len() {
            let status = regs.read32(CS);
            let mut progressed = false;

            if sent < out.len() && status & CS_TXD != 0 {
                regs.write32(FIFO, u32::from(out[sent]));
                sent += 1;
                progressed = true;
            }

            if recv < sent && status & CS_RXD != 0 {
                inp[recv] = regs.read32(FIFO) as u8;
                recv += 1;
                progressed = true;
            }

            // The budget counts *consecutive* stalls, not total iterations: a
            // long transfer is not a hang, and must not be timed out as one.
            if progressed {
                idle = 0;
            } else {
                idle += 1;
                if idle > SPIN_LIMIT {
                    return Err(BcmSpiError::Timeout);
                }
            }
        }

        Ok(())
    }

    /// Run one framed transaction: `TA` raised, `body`, `TA` dropped.
    ///
    /// `stop` runs whether or not `body` succeeded. Returning early from a
    /// failed transfer used to leave `TA` asserted and the RX FIFO holding
    /// stale bytes — the bus-level version of leaving chip-select low, which
    /// [`super::ExclusiveDevice`] is careful never to do.
    fn framed(
        &self,
        body: impl FnOnce(&Self) -> Result<(), BcmSpiError>,
    ) -> Result<(), BcmSpiError> {
        self.start();
        let transferred = body(self);
        let stopped = self.stop();
        // A transfer error is the more informative one; a stop error only says
        // the block was already wedged, which the transfer error implies.
        match (transferred, stopped) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn start(&self) {
        // Clear FIFOs, then raise TA. Mode 0 bits remain zero.
        self.regs.write32(CS, CS_CLEAR);
        self.regs.write32(CS, CS_TA);
    }

    fn stop(&self) -> Result<(), BcmSpiError> {
        let regs = self.regs;
        let outcome = self.quiesce();
        // Drop TA and clear the FIFOs on every path, including the timeout
        // above: this is the call that has to leave the block idle, so it
        // cannot be the one that returns early.
        regs.write32(CS, CS_CLEAR);
        regs.write32(CS, 0);
        outcome
    }

    /// Drain residual RX and wait for DONE. Bounded like every other wait.
    fn quiesce(&self) -> Result<(), BcmSpiError> {
        let regs = self.regs;
        let mut spins = 0u32;
        while regs.read32(CS) & CS_RXD != 0 {
            let _ = regs.read32(FIFO);
            spins = spins.saturating_add(1);
            if spins > SPIN_LIMIT {
                return Err(BcmSpiError::Timeout);
            }
        }
        self.wait_cs(CS_DONE)
    }
}

impl SpiBus for BcmSpi {
    type Error = BcmSpiError;

    fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        self.framed(|spi| {
            // MISO still has to be clocked in and dropped: the block will not
            // advance once the RX FIFO fills, so "write only" still reads.
            let mut sink = [0u8; FIFO_DEPTH];
            for chunk in words.chunks(FIFO_DEPTH) {
                spi.transfer_chunk(chunk, &mut sink[..chunk.len()])?;
            }
            Ok(())
        })
    }

    fn transfer(&mut self, read: &mut [u8], words: &[u8]) -> Result<(), Self::Error> {
        if read.len() != words.len() {
            return Err(BcmSpiError::LengthMismatch);
        }
        self.framed(|spi| {
            for (rx, tx) in read.chunks_mut(FIFO_DEPTH).zip(words.chunks(FIFO_DEPTH)) {
                spi.transfer_chunk(tx, rx)?;
            }
            Ok(())
        })
    }

    fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        self.framed(|spi| {
            // The outgoing bytes have to survive being overwritten by the
            // incoming ones, so each chunk is copied out before it is clocked.
            let mut outgoing = [0u8; FIFO_DEPTH];
            for chunk in words.chunks_mut(FIFO_DEPTH) {
                let staged = &mut outgoing[..chunk.len()];
                staged.copy_from_slice(chunk);
                spi.transfer_chunk(staged, chunk)?;
            }
            Ok(())
        })
    }
}
