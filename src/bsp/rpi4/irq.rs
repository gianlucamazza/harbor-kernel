//! Board IRQ bind: GIC-400 instance + timer PPI + UART0 SPI + wake SGI.

use crate::arch::{cpu, smp, timer};
use crate::bsp::rpi4::memmap::{GICC_BASE, GICD_BASE, TIMER_PPI, UART0_SPI};
use crate::console;
use crate::drivers::gicv2::GicV2;
use crate::irq;
use crate::time;

/// Platform GIC — single distributor owner: core 0 programs GICD; each core
/// that takes IRQs programs its banked GICC (ADR-0070/0074).
// SAFETY: `GICD_BASE` / `GICC_BASE` are this board's distributor and CPU
// interface, compiled in rather than read from the device tree (ADR-0011), and
// both are inside the GIC region `mm::layout` maps Device-nGnRnE. A `static`
// so there is exactly one handle: a second would be a second owner of the same
// registers, which the `Mmio` contract forbids.
static GIC: GicV2 = unsafe { GicV2::new(GICD_BASE, GICC_BASE) };

/// Arch timer IRQ line on this board (CNTP NS → PPI 30).
pub const TIMER_IRQ: u32 = TIMER_PPI;

/// PL011 UART0 RX (and other UART events) on this board — GIC SPI 153.
pub const UART_IRQ: u32 = UART0_SPI;

/// Software-generated wake line for core 1 (ADR-0074). Banked; enabled only
/// on the secondary.
pub const WAKE_SGI: u32 = 0;

/// CPU interface bit for core 1 in `GICD_SGIR.CPUTargetList`.
const CORE1_TARGET_BIT: u8 = 1 << 1;

/// Spin budget for secondary IRQ ready / IPI flags (~same order as unpark).
const SECONDARY_SPIN_BUDGET: u64 = 200_000_000;

/// Initialise irqchip, register timer + UART + wake-SGI handlers, program
/// timer, enable lines.
///
/// IRQs must remain **masked** until bootstrap finishes soft proof and arms
/// PL011 `IMSC` via [`console::enable_rx_irq`].
///
/// Everything that can go wrong binding the board's interrupt sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    /// A handler could not be registered: the line would be acknowledged and
    /// dropped, so the console would silently receive nothing.
    ///
    /// Carries the reason rather than only the id. The two are different
    /// failures with different fixes — an id past the table is a constant to
    /// correct, while a sealed table is a bring-up ordering bug — and this
    /// error is what a refusal to boot prints.
    HandlerNotRegistered(irq::RegisterError),
    /// The arch timer would not start at the requested rate.
    Timer(timer::TimerError),
}

/// # Safety
/// Primary core; exclusive GIC distributor ownership; call once.
pub unsafe fn init(timer_hz: u32) -> Result<(), BindError> {
    // SAFETY: the caller guarantees this runs once, on the primary core, with
    // IRQs masked — which is what makes registering handlers and then sealing
    // the table a sequence no interrupt can observe half-done.
    unsafe {
        irq::init(&GIC);

        // Cookies are kernel-internal ids (ADR-0008), not GIC numbers.
        if let Err(e) = irq::register(TIMER_IRQ, time::on_timer_irq, 1) {
            return Err(BindError::HandlerNotRegistered(e));
        }
        if let Err(e) = irq::register(UART_IRQ, console::on_uart_rx_irq, 2) {
            return Err(BindError::HandlerNotRegistered(e));
        }
        // Wake SGI: registered on the shared table so core 1's claim path
        // finds a handler. Enabled only on the secondary (banked ISENABLER).
        if let Err(e) = irq::register(WAKE_SGI, on_wake_sgi, 3) {
            return Err(BindError::HandlerNotRegistered(e));
        }

        timer::init(timer_hz).map_err(BindError::Timer)?;

        irq::enable(TIMER_IRQ);
        irq::enable(UART_IRQ);
        Ok(())
    }
}

/// Primary: release secondary IRQ bring-up, wait ready, send SGI 0, wait flag.
///
/// Returns whether core 1 handled the IPI. Call only when core 1 is alive and
/// the dispatch table is sealed (handler registered).
pub fn probe_core1_ipi() -> bool {
    smp::release_secondary_irq_bringup();
    if !smp::wait_secondary_irq_ready(SECONDARY_SPIN_BUDGET) {
        return false;
    }
    if !send_resched_sgi() {
        return false;
    }
    smp::wait_core1_ipi(SECONDARY_SPIN_BUDGET)
}

/// Send the wake/resched SGI to CPU 1 (ADR-0074/0076).
#[inline]
pub fn send_resched_sgi() -> bool {
    GIC.send_sgi_raw(WAKE_SGI, CORE1_TARGET_BIT)
}

/// Shared-table handler for the wake SGI (ADR-0074 / resched ADR-0076).
fn on_wake_sgi(_cookie: irq::IrqCookie) {
    // Only core 1 is expected to receive this line; still gate the flag so a
    // mis-targeted delivery on CPU0 does not forge the oracle.
    if cpu::affinity() == 1 {
        smp::note_core1_ipi();
        // Handler-safe resched poke (no sched lock — ADR-0075). Lives in
        // `arch::smp` so this board module never imports `sched` (layering).
        smp::request_resched(1);
    }
}

/// Core 1 idle after MMU/VBAR/alive (ADR-0070/0074).
///
/// Linked by symbol from `arch::smp::secondary_main` so arch never imports
/// this board module. Waits for the primary's GICD + seal, programs banked
/// GICC, enables SGI 0, unmasks IRQs, then `WFI` forever.
///
/// # Safety
/// Runs only on affinity 1 with IRQs masked on entry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn harbor_secondary_idle() -> ! {
    while !smp::secondary_may_irq() {
        cpu::wait_for_event();
    }

    // Banked GICC + SGI/PPI Group 0 on this core; distributor already open.
    GIC.init_this_cpu();
    // Priority + group + banked enable for SGI 0 only (no timer on secondary).
    // Goes through the shared chip pointer so the secondary does not need a
    // second `GicV2` owner of the same MMIO.
    irq::enable(WAKE_SGI);
    smp::mark_secondary_irq_ready();

    cpu::sync_pipeline();
    cpu::irq_enable();
    cpu::sync_pipeline();

    // Schedule policy lives in sched (ADR-0076); board only brought up GIC.
    unsafe extern "C" {
        fn harbor_secondary_sched() -> !;
    }
    // SAFETY: affinity 1, IRQs unmasked, banked GIC ready.
    unsafe { harbor_secondary_sched() }
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
