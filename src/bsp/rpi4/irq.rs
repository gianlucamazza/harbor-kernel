//! Board IRQ bind: GIC-400 instance + timer PPI + UART0 SPI wiring.

use crate::arch::timer;
use crate::bsp::rpi4::memmap::{GICC_BASE, GICD_BASE, TIMER_PPI, UART0_SPI};
use crate::console;
use crate::drivers::gicv2::GicV2;
use crate::irq;
use crate::time;

/// Platform GIC — single owner for M1 (core 0 only).
static GIC: GicV2 = unsafe { GicV2::new(GICD_BASE, GICC_BASE) };

/// Arch timer IRQ line on this board (CNTP NS → PPI 30).
pub const TIMER_IRQ: u32 = TIMER_PPI;

/// PL011 UART0 RX (and other UART events) on this board — GIC SPI 153.
pub const UART_IRQ: u32 = UART0_SPI;

/// Initialise irqchip, register timer + UART handlers, program timer, enable lines.
///
/// IRQs must remain **masked** until bootstrap finishes soft proof and arms
/// PL011 `IMSC` via [`console::enable_rx_irq`].
///
/// # Safety
/// Single core; exclusive GIC ownership; call once.
/// Returns `false` if a handler could not be registered — the line would then
/// be acknowledged and dropped, so the caller should say so rather than boot
/// into a console that silently never receives anything.
#[must_use]
pub unsafe fn init(timer_hz: u32) -> bool {
    unsafe {
        irq::init(&GIC);

        let registered = irq::register(TIMER_IRQ, time::on_timer_irq)
            & irq::register(UART_IRQ, console::on_uart_rx_irq);

        timer::init(timer_hz);
        irq::enable(TIMER_IRQ);
        irq::enable(UART_IRQ);

        registered
    }
}

/// Raw `GICC_IAR` read (side-effect: claim). Bring-up / selftest only.
#[allow(dead_code)]
pub fn debug_read_iar() -> u32 {
    GIC.debug_iar()
}

/// Raw `GICC_HPPIR` read (no claim). Bring-up / selftest only.
#[allow(dead_code)]
pub fn debug_read_hppir() -> u32 {
    GIC.debug_hppir()
}

/// Raw `GICC_EOIR` write. Bring-up / selftest only.
#[allow(dead_code)]
pub fn debug_write_eoir(val: u32) {
    GIC.debug_eoir(val);
}
