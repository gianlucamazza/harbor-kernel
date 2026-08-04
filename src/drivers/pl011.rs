//! ARM PrimeCell PL011 UART driver (polling, no IRQ).
//!
//! Board-agnostic: the caller supplies the MMIO base and clock rate.
//! Reference: ARM DDI 0183 (PL011 Technical Reference Manual).

use core::fmt;

use crate::arch::mmio::Mmio;

// Register offsets (bytes).
const DR: usize = 0x00;
const FR: usize = 0x18;
const IBRD: usize = 0x24;
const FBRD: usize = 0x28;
const LCRH: usize = 0x2C;
const CR: usize = 0x30;
const ICR: usize = 0x44;

// FR bits.
const FR_RXFE: u32 = 1 << 4;
const FR_TXFF: u32 = 1 << 5;

// CR bits.
const CR_UARTEN: u32 = 1 << 0;
const CR_TXE: u32 = 1 << 8;
const CR_RXE: u32 = 1 << 9;

// LCRH bits.
const LCRH_FEN: u32 = 1 << 4;
const LCRH_WLEN_8BIT: u32 = 0b11 << 5;

/// Line format used by this driver.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// UART reference clock in Hz (board-specific).
    pub clock_hz: u32,
    /// Desired baud rate.
    pub baud: u32,
}

impl Config {
    /// Compute integer and fractional baud divisors.
    ///
    /// Formula (PL011): `baud = clock / (16 * (IBRD + FBRD/64))`.
    pub const fn divisors(self) -> (u32, u32) {
        // divisor_x64 = 64 * clock / (16 * baud) = 4 * clock / baud
        let divisor_x64 = (4 * self.clock_hz) / self.baud;
        let ibrd = divisor_x64 >> 6;
        let fbrd = divisor_x64 & 0x3F;
        (ibrd, fbrd)
    }
}

/// Polling PL011 instance.
pub struct Pl011 {
    regs: Mmio,
}

impl Pl011 {
    /// Bind a PL011 at `mmio` and program it according to `config`.
    ///
    /// Safe to call more than once on the same hardware: each call fully
    /// re-initialises the controller (required for a correct panic path).
    ///
    /// # Safety
    ///
    /// `mmio` must address a PL011 register block exclusive to this driver
    /// for the duration of use.
    pub unsafe fn init(mmio: Mmio, config: Config) -> Self {
        let uart = Self { regs: mmio };
        uart.configure(config);
        uart
    }

    fn configure(&self, config: Config) {
        let regs = self.regs;
        let (ibrd, fbrd) = config.divisors();

        // Disable before touching divisors / line control.
        regs.write32(CR, 0);
        regs.write32(ICR, 0x7FF);
        regs.write32(IBRD, ibrd);
        regs.write32(FBRD, fbrd);
        regs.write32(LCRH, LCRH_WLEN_8BIT | LCRH_FEN);
        regs.write32(CR, CR_UARTEN | CR_TXE | CR_RXE);
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

    /// Non-blocking receive: `None` if the RX FIFO is empty.
    pub fn read_byte(&self) -> Option<u8> {
        if self.regs.read32(FR) & FR_RXFE != 0 {
            None
        } else {
            Some((self.regs.read32(DR) & 0xFF) as u8)
        }
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
