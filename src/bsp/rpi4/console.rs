//! Board-level serial console: pinmux + PL011 UART0.

use kernel_core::uart::{BaudConfig, Divisors};

use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::{gpio, memmap};
use crate::drivers::pl011::Pl011;

/// Console line rate requested from the board UART clock.
const CONSOLE_RATE: BaudConfig = BaudConfig {
    clock_hz: memmap::UART0_CLOCK_HZ,
    baud: memmap::UART0_BAUD,
};

/// Divisors resolved at compile time.
///
/// A rate the hardware cannot program is a build failure rather than a board
/// that boots and prints noise — the failure mode that is hardest to diagnose
/// over a serial line is a serial line that lies.
const CONSOLE_DIVISORS: Divisors = match CONSOLE_RATE.divisors() {
    Some(divisors) => divisors,
    None => panic!("console baud rate is not programmable at the board UART clock"),
};

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

    // SAFETY: `UART0_BASE` is the PL011 register block on BCM2711.
    Pl011::init(mmio, CONSOLE_DIVISORS)
}
