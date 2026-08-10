//! 16550 UART — polled TX for x86 lab (ADR-0071).

use core::fmt;

use crate::arch::mmio;

/// Polled 16550 on an I/O port base (e.g. COM1 = 0x3F8).
pub struct Uart16550 {
    base: u16,
}

impl Uart16550 {
    /// # Safety
    /// `base` must be a live 16550 port block.
    pub unsafe fn new(base: u16) -> Self {
        let u = Self { base };
        // SAFETY: programming the UART.
        unsafe {
            u.init();
        }
        u
    }

    unsafe fn init(&self) {
        // 8N1, no IRQ, DLAB for baud then clear.
        // SAFETY: port I/O to COM1.
        unsafe {
            mmio::outb(self.base + 1, 0x00); // disable IRQs
            mmio::outb(self.base + 3, 0x80); // DLAB
            mmio::outb(self.base + 0, 0x01); // divisor low (115200)
            mmio::outb(self.base + 1, 0x00); // divisor high
            mmio::outb(self.base + 3, 0x03); // 8N1
            mmio::outb(self.base + 2, 0xC7); // FIFO
            mmio::outb(self.base + 4, 0x0B); // IRQs off, RTS/DSR
        }
    }

    pub fn write_byte(&self, b: u8) {
        // SAFETY: wait for THR empty then write.
        unsafe {
            while mmio::inb(self.base + 5) & 0x20 == 0 {}
            mmio::outb(self.base, b);
        }
    }
}

impl fmt::Write for Uart16550 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(b);
        }
        Ok(())
    }
}
