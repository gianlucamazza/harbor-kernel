//! BCM2711 GPIO — only the pinmux required for the serial console.
//!
//! GPIO 14 = TXD0 (ALT0), GPIO 15 = RXD0 (ALT0).

use crate::arch::mmio::{Mmio, spin_cycles};
use crate::bsp::rpi4::memmap::GPIO_BASE;

const GPFSEL1: usize = 0x04;
const GPPUPPDN0: usize = 0xE4;

const ALT0: u32 = 0b100;
const PULL_NONE: u32 = 0b00;

/// Configure GPIO 14/15 for PL011 UART0 and disable pad pull.
///
/// # Safety
///
/// Must run with exclusive ownership of the GPIO controller (true at early
/// boot on a single active core before any other driver touches GPIO).
pub unsafe fn configure_uart0_pins() {
    unsafe {
        let gpio = Mmio::new(GPIO_BASE);

        // GPFSEL1: GPIO 10–19, 3 bits per pin.
        // GPIO14 → [14:12], GPIO15 → [17:15].
        let mut fsel = gpio.read32(GPFSEL1);
        fsel &= !((0b111 << 12) | (0b111 << 15));
        fsel |= (ALT0 << 12) | (ALT0 << 15);
        gpio.write32(GPFSEL1, fsel);

        // GPPUPPDN0: GPIO 0–15, 2 bits per pin (BCM2711).
        // GPIO14 → [29:28], GPIO15 → [31:30].
        let mut pull = gpio.read32(GPPUPPDN0);
        pull &= !((0b11 << 28) | (0b11 << 30));
        pull |= (PULL_NONE << 28) | (PULL_NONE << 30);
        gpio.write32(GPPUPPDN0, pull);

        // Brief settle for pad configuration.
        spin_cycles(150);
    }
}
