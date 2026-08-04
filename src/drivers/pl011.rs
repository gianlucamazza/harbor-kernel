//! ARM PrimeCell PL011 UART driver.
//!
//! Board-agnostic: the caller supplies the MMIO base and the pre-computed baud
//! divisors. Reference: ARM DDI 0183 (PL011 Technical Reference Manual).
//!
//! TX is still polled. RX can be polled or interrupt-driven (`RXIM` + `RTIM`).
//! Divisor arithmetic lives in [`kernel_core::uart`] so it is unit-tested on
//! the host; this module only owns the register sequence.

use core::fmt;

use kernel_core::uart::Divisors;

use crate::arch::mmio::Mmio;

// Register offsets (bytes).
const DR: usize = 0x00;
const FR: usize = 0x18;
const IBRD: usize = 0x24;
const FBRD: usize = 0x28;
const LCRH: usize = 0x2C;
const CR: usize = 0x30;
const IMSC: usize = 0x38;
const ICR: usize = 0x44;

// FR bits.
const FR_BUSY: u32 = 1 << 3;
const FR_RXFE: u32 = 1 << 4;
const FR_TXFF: u32 = 1 << 5;

// DR error bits (framing, parity, break, overrun).
const DR_ERRORS: u32 = 0b1111 << 8;

// CR bits.
const CR_UARTEN: u32 = 1 << 0;
const CR_TXE: u32 = 1 << 8;
const CR_RXE: u32 = 1 << 9;

// LCRH bits.
const LCRH_FEN: u32 = 1 << 4;
const LCRH_WLEN_8BIT: u32 = 0b11 << 5;

// IMSC / ICR bits (receive data + receive timeout for single-char with FIFO).
const IMSC_RXIM: u32 = 1 << 4;
const IMSC_RTIM: u32 = 1 << 6;
const ICR_RXIC: u32 = 1 << 4;
const ICR_RTIC: u32 = 1 << 6;

/// Upper bound on the spin waiting for an in-flight character to drain.
///
/// The panic path re-programs the UART and must never hang there, so the wait
/// is bounded: a wedged transmitter costs one garbled character, not the
/// diagnostic that follows it.
const BUSY_SPIN_LIMIT: u32 = 100_000;

/// PL011 instance (TX polled; RX polled or IRQ).
pub struct Pl011 {
    regs: Mmio,
}

impl Pl011 {
    /// Bind a PL011 at `mmio` and program it with `divisors`.
    ///
    /// Safe to call more than once on the same hardware: each call fully
    /// re-initialises the controller (required for a correct panic path).
    ///
    /// # Safety
    ///
    /// `mmio` must address a PL011 register block exclusive to this driver
    /// for the duration of use.
    pub unsafe fn init(mmio: Mmio, divisors: Divisors) -> Self {
        let uart = Self { regs: mmio };
        uart.configure(divisors);
        uart
    }

    /// Wrap an already-programmed PL011 MMIO window (no register writes).
    ///
    /// # Safety
    ///
    /// `mmio` must address a live PL011 that another owner initialised; used
    /// for a second handle (RX IRQ path) on the same hardware.
    #[inline]
    pub const unsafe fn from_mmio(mmio: Mmio) -> Self {
        Self { regs: mmio }
    }

    /// MMIO window (Copy) for a second owner such as the RX IRQ path.
    #[inline]
    pub fn mmio(&self) -> Mmio {
        self.regs
    }

    fn configure(&self, divisors: Divisors) {
        let regs = self.regs;

        // Let any character already in flight finish. Disabling mid-character
        // corrupts it, which matters precisely when the panic handler takes the
        // console away from a main loop that was writing.
        let mut spins = 0;
        while regs.read32(FR) & FR_BUSY != 0 && spins < BUSY_SPIN_LIMIT {
            spins += 1;
        }

        // Disable, then flush the FIFO by clearing FEN before touching the
        // divisors (DDI 0183: LCRH must be rewritten after IBRD/FBRD).
        regs.write32(CR, 0);
        let lcrh = regs.read32(LCRH);
        regs.write32(LCRH, lcrh & !LCRH_FEN);

        regs.write32(IMSC, 0);
        regs.write32(ICR, 0x7FF);
        regs.write32(IBRD, divisors.ibrd);
        regs.write32(FBRD, divisors.fbrd);
        regs.write32(LCRH, LCRH_WLEN_8BIT | LCRH_FEN);
        regs.write32(CR, CR_UARTEN | CR_TXE | CR_RXE);
    }

    /// Enable RX data + receive-timeout interrupts (FIFO-friendly single char).
    pub fn enable_rx_interrupt(&self) {
        self.regs.write32(IMSC, IMSC_RXIM | IMSC_RTIM);
    }

    /// Clear RX and receive-timeout interrupt status bits.
    pub fn clear_rx_interrupt(&self) {
        self.regs.write32(ICR, ICR_RXIC | ICR_RTIC);
    }

    /// Transmit one byte, blocking while the TX FIFO is full.
    pub fn write_byte(&self, byte: u8) {
        while self.regs.read32(FR) & FR_TXFF != 0 {}
        self.regs.write32(DR, u32::from(byte));
    }

    /// Transmit raw bytes (no newline translation).
    pub fn write_bytes(&self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_byte(byte);
        }
    }

    /// Non-blocking receive: `None` if the RX FIFO is empty or the character
    /// arrived with a framing/parity/break/overrun error.
    pub fn read_byte(&self) -> Option<u8> {
        if self.regs.read32(FR) & FR_RXFE != 0 {
            return None;
        }
        // Reading DR pops the FIFO; the error flags belong to this character.
        let dr = self.regs.read32(DR);
        if dr & DR_ERRORS != 0 {
            return None;
        }
        Some((dr & 0xFF) as u8)
    }

    /// Drain the RX FIFO, invoking `push` for each good byte.
    ///
    /// Error characters are discarded (still popped). If `push` returns
    /// `false` (e.g. ring full), the byte is dropped and draining continues so
    /// a level-triggered line does not re-fire forever. Clears RX/RT status.
    ///
    /// Safe for the IRQ path: no TX, no formatting.
    pub fn drain_rx(&self, mut push: impl FnMut(u8) -> bool) {
        while self.regs.read32(FR) & FR_RXFE == 0 {
            let dr = self.regs.read32(DR);
            if dr & DR_ERRORS == 0 {
                let _ = push((dr & 0xFF) as u8);
            }
        }
        self.clear_rx_interrupt();
    }
}

impl fmt::Write for Pl011 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
        Ok(())
    }
}
