//! ARM PrimeCell PL011 UART driver.
//!
//! Board-agnostic: the caller supplies the MMIO base and the pre-computed baud
//! divisors. Reference: ARM DDI 0183 (PL011 Technical Reference Manual).
//!
//! TX is still polled. RX can be polled or interrupt-driven (`RXIM` + `RTIM`).
//! Divisor arithmetic lives in [`kernel_core::uart`] so it is unit-tested on
//! the host; this module only owns the register sequence.

use core::fmt;

use kernel_core::poll;
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
/// Loopback enable (DDI 0183): TX is fed to RX inside the block.
const CR_LBE: u32 = 1 << 7;
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
/// diagnostic that follows it. The budget behaviour itself is tested in
/// [`kernel_core::poll`], where a condition that never clears can be produced
/// on demand — which real hardware will not do when asked.
const BUSY_SPIN_LIMIT: u32 = 100_000;

/// Upper bound on the spin waiting for room in the TX FIFO.
///
/// Generous: at 115200 baud a full 16-byte FIFO drains in about 1.4 ms, and a
/// CPU at 1.5 GHz spins this many times in a few milliseconds. Only a UART
/// that has stopped transmitting altogether reaches it.
const TX_SPIN_LIMIT: u32 = 10_000_000;

/// The console owner: configuration plus transmit.
///
/// Deliberately cannot receive. The receive half is a separate type handed to
/// the IRQ path by [`Pl011::receiver`], so the two capabilities are disjoint by
/// construction rather than by a comment asking each side to behave.
///
/// The hardware register block is still shared — that is what the PL011 is —
/// but the sharing is exactly what the device supports: a `DR` write pushes
/// the transmit FIFO, a `DR` read pops the receive FIFO, and `FR` is
/// read-only status. What the split removes is the ability of the IRQ handler
/// to transmit (rule 6 in docs/architecture.md, previously enforced only by
/// convention) and of the transmitter to disturb interrupt configuration.
pub struct Pl011 {
    regs: Mmio,
}

/// The receive half: drains the FIFO and acknowledges, nothing else.
///
/// `Copy` so it can be reconstructed in the IRQ handler from a base address
/// published atomically — the handler must not chase a pointer the main loop
/// could be halfway through writing.
#[derive(Clone, Copy)]
pub struct Pl011Rx {
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

    /// The receive half of this UART, for the IRQ path.
    ///
    /// Safe: the returned type can only drain and acknowledge receive, which
    /// is disjoint from everything this handle does.
    #[inline]
    pub fn receiver(&self) -> Pl011Rx {
        Pl011Rx { regs: self.regs }
    }

    fn configure(&self, divisors: Divisors) {
        let regs = self.regs;

        // Let any character already in flight finish. Disabling mid-character
        // corrupts it, which matters precisely when the panic handler takes the
        // console away from a main loop that was writing.
        poll::until(BUSY_SPIN_LIMIT, || regs.read32(FR) & FR_BUSY == 0);

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

    /// Mask all PL011 IRQs (agent poll ownership; kernel drain is off).
    pub fn disable_rx_interrupt(&self) {
        self.regs.write32(IMSC, 0);
    }

    /// Enable or clear internal TX→RX loopback (self-test without host input).
    ///
    /// Does not touch baud/format. Caller must own exclusive use of RX for the
    /// window (kernel drain suspended) so looped bytes are not stolen by IRQ.
    pub fn set_loopback(&self, on: bool) {
        let mut cr = self.regs.read32(CR);
        if on {
            cr |= CR_LBE;
        } else {
            cr &= !CR_LBE;
        }
        self.regs.write32(CR, cr);
    }

    /// Transmit one byte, waiting for room in the TX FIFO.
    ///
    /// Returns `false` if the FIFO never drained and the byte was dropped.
    ///
    /// The wait is bounded, and that matters most on the path that cannot
    /// afford to hang: the panic handler masks interrupts, takes the console
    /// and writes. If the UART is wedged — no receiver asserting flow control,
    /// a clock that stopped, a half-programmed controller — an unbounded spin
    /// there means the board goes quiet at exactly the moment it has something
    /// to say. A dropped character is a bad diagnostic; a hang is none.
    pub fn write_byte(&self, byte: u8) -> bool {
        if !poll::until(TX_SPIN_LIMIT, || self.regs.read32(FR) & FR_TXFF == 0) {
            return false;
        }
        self.regs.write32(DR, u32::from(byte));
        true
    }

    /// Transmit raw bytes (no newline translation).
    ///
    /// Stops at the first byte that could not be sent: once the FIFO has
    /// stopped draining, the rest of the line will not fare better, and
    /// retrying each byte multiplies the stall by the message length.
    pub fn write_bytes(&self, bytes: &[u8]) -> bool {
        for &byte in bytes {
            if !self.write_byte(byte) {
                return false;
            }
        }
        true
    }
}

impl Pl011Rx {
    /// Rebuild the receive half from a base address.
    ///
    /// # Safety
    ///
    /// `base` must be a PL011 register block already programmed by a
    /// [`Pl011`] owner. Exists because the IRQ handler reads the base from an
    /// atomic rather than from a shared handle it could observe half-written.
    #[inline]
    pub const unsafe fn from_base(base: usize) -> Self {
        Self {
            // SAFETY: forwarded from the caller's obligation.
            regs: unsafe { Mmio::new(base) },
        }
    }

    /// Base address of the register block, for publishing through an atomic.
    #[inline]
    pub fn base(&self) -> usize {
        self.regs.base()
    }

    /// Clear RX and receive-timeout interrupt status bits.
    fn clear_interrupt(&self) {
        self.regs.write32(ICR, ICR_RXIC | ICR_RTIC);
    }

    /// Drop any pending RX bytes and clear the RX/RT interrupt line.
    ///
    /// Used when kernel drain is suspended so an EL0 agent can poll `DR`
    /// without a level-triggered UART SPI re-firing into a no-op handler.
    pub fn discard_and_ack(&self) {
        while self.regs.read32(FR) & FR_RXFE == 0 {
            let _ = self.regs.read32(DR);
        }
        self.clear_interrupt();
    }

    /// Non-blocking receive: `None` if the RX FIFO is empty or the character
    /// arrived with a framing/parity/break/overrun error.
    ///
    /// Production RX is interrupt-driven ([`Self::drain`]); this is the polled
    /// path the bring-up soft console falls back to.
    #[cfg(feature = "bringup")]
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
    /// Safe for the IRQ path: this type cannot transmit or format.
    pub fn drain(&self, mut push: impl FnMut(u8) -> bool) {
        while self.regs.read32(FR) & FR_RXFE == 0 {
            let dr = self.regs.read32(DR);
            if dr & DR_ERRORS == 0 {
                let _ = push((dr & 0xFF) as u8);
            }
        }
        self.clear_interrupt();
    }
}

impl fmt::Write for Pl011 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' && !self.write_byte(b'\r') {
                return Err(fmt::Error);
            }
            if !self.write_byte(byte) {
                // A wedged transmitter is reported rather than spun on. Every
                // caller uses `let _ = write!(...)`, so this degrades to lost
                // output instead of a hang.
                return Err(fmt::Error);
            }
        }
        Ok(())
    }
}
