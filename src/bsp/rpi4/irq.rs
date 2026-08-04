//! Board IRQ bind: GIC-400 instance + timer PPI wiring.

use crate::arch::timer;
use crate::bsp::rpi4::memmap::{GICC_BASE, GICD_BASE, TIMER_PPI};
use crate::drivers::gicv2::GicV2;
use crate::irq;
use crate::time;

/// Platform GIC — single owner for M1 (core 0 only).
static GIC: GicV2 = unsafe { GicV2::new(GICD_BASE, GICC_BASE) };

/// Arch timer IRQ line on this board (CNTP NS → PPI 30).
pub const TIMER_IRQ: u32 = TIMER_PPI;

/// Initialise irqchip, register timer handler, program timer, enable PPI.
///
/// IRQs must remain **masked** until bootstrap finishes soft proof.
///
/// # Safety
/// Single core; exclusive GIC ownership; call once.
pub unsafe fn init(timer_hz: u32) {
    irq::init(&GIC);
    irq::register(TIMER_IRQ, time::on_timer_irq);
    timer::init(timer_hz);
    irq::enable(TIMER_IRQ);
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
