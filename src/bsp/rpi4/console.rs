//! Board-level serial console: pinmux + PL011 UART0.

use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::{gpio, memmap};
use crate::drivers::pl011::{Config, Pl011};

/// Bring up the board serial console and return a ready PL011 handle.
///
/// Idempotent with respect to the UART controller: calling again re-programs
/// pinmux and the PL011 from a known state (used by the panic path).
///
/// # Safety
///
/// Exclusive access to GPIO and UART0 MMIO is required. On M0 this holds
/// because only core 0 runs and no other subsystem touches these devices.
pub unsafe fn init() -> Pl011 {
    gpio::configure_uart0_pins();

    let mmio = Mmio::new(memmap::UART0_BASE);
    let config = Config {
        clock_hz: memmap::UART0_CLOCK_HZ,
        baud: memmap::UART0_BAUD,
    };

    // SAFETY: `UART0_BASE` is the PL011 register block on BCM2711.
    Pl011::init(mmio, config)
}
