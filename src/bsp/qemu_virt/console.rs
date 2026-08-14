//! QEMU `virt` PL011 console bind.

use kernel_core::uart::{BaudConfig, Divisors};

use crate::arch::mmio::Mmio;
use crate::bsp::qemu_virt::memmap;
use crate::drivers::pl011::Pl011;

const RATE: BaudConfig = BaudConfig {
    clock_hz: memmap::UART0_CLOCK_HZ,
    baud: memmap::UART0_BAUD,
};

const DIVISORS: Divisors = match RATE.divisors() {
    Some(divisors) => divisors,
    None => panic!("QEMU virt PL011 rate is not programmable"),
};

/// Initialise and return the exclusive QEMU PL011 handle.
///
/// # Safety
/// The caller owns the UART and the MMIO window is Device-mapped.
pub unsafe fn init() -> Pl011 {
    // SAFETY: the board bind owns the only PL011 handle.
    unsafe { Pl011::init(Mmio::new(memmap::UART0_BASE), DIVISORS) }
}
