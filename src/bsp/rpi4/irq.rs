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
/// Everything that can go wrong binding the board's interrupt sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    /// A handler id was beyond the dispatch table: the line would be
    /// acknowledged and dropped, so the console would silently receive nothing.
    HandlerNotRegistered(u32),
    /// The arch timer would not start at the requested rate.
    Timer(timer::TimerError),
}

/// # Safety
/// Single core; exclusive GIC ownership; call once.
pub unsafe fn init(timer_hz: u32) -> Result<(), BindError> {
    unsafe {
        irq::init(&GIC);

        if !irq::register(TIMER_IRQ, time::on_timer_irq) {
            return Err(BindError::HandlerNotRegistered(TIMER_IRQ));
        }
        if !irq::register(UART_IRQ, console::on_uart_rx_irq) {
            return Err(BindError::HandlerNotRegistered(UART_IRQ));
        }

        timer::init(timer_hz).map_err(BindError::Timer)?;

        irq::enable(TIMER_IRQ);
        irq::enable(UART_IRQ);
        Ok(())
    }
}

/// Raw GIC accessors for the bring-up gates.
///
/// Compiled only with the `bringup` feature: these have side effects on the
/// interrupt controller and exist to debug it, not to be used by kernel policy.
#[cfg(feature = "bringup")]
mod debug {
    use super::GIC;

    /// Highest pending id without claiming it.
    pub fn debug_peek_pending() -> Option<u32> {
        GIC.debug_hppir_id()
    }

    /// Raw `GICC_IAR` read (side-effect: claim).
    pub fn debug_read_iar() -> u32 {
        GIC.debug_iar()
    }

    /// Raw `GICC_EOIR` write.
    pub fn debug_write_eoir(val: u32) {
        GIC.debug_eoir(val);
    }
}

#[cfg(feature = "bringup")]
pub use debug::*;
