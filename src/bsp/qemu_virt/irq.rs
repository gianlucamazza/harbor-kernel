//! QEMU `virt` GICv2 binding for the shared kernel IRQ policy.

use crate::arch::{cpu, smp, timer};
use crate::bsp::qemu_virt::memmap::{
    GICC_BASE, GICD_BASE, TIMER_PPI, UART0_SPI, VIRTIO_MMIO_SLOTS, VIRTIO_MMIO_STRIDE,
    VIRTIO_NET_BASE,
};
use crate::console;
use crate::drivers::gicv2::GicV2;
use crate::drivers::virtio_mmio;
use crate::irq;
use crate::time;

// SAFETY: these are QEMU's GICD/GICC windows for the selected GICv2 machine;
// this static is the sole owner of the controller registers.
static GIC: GicV2 = unsafe { GicV2::new(GICD_BASE, GICC_BASE) };

pub const TIMER_IRQ: u32 = TIMER_PPI;
pub const UART_IRQ: u32 = UART0_SPI;
pub const WAKE_SGI: u32 = 0;
const CORE1_TARGET_BIT: u8 = 1 << 1;
const SECONDARY_SPIN_BUDGET: u64 = 200_000_000;
const VIRTIO_NET_SPI: u32 = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    HandlerNotRegistered(irq::RegisterError),
    Timer(timer::TimerError),
}

/// Bind timer, PL011 RX, and the scheduler wake SGI.
///
/// # Safety
/// Primary core, IRQs masked, exclusive GIC ownership.
pub unsafe fn init(timer_hz: u32) -> Result<(), BindError> {
    // SAFETY: caller provides the single-core bring-up preconditions.
    unsafe {
        irq::init(&GIC);
        irq::register(TIMER_IRQ, time::on_timer_irq, 1).map_err(BindError::HandlerNotRegistered)?;
        irq::register(UART_IRQ, console::on_uart_rx_irq, 2)
            .map_err(BindError::HandlerNotRegistered)?;
        irq::register(WAKE_SGI, on_wake_sgi, 3).map_err(BindError::HandlerNotRegistered)?;
        for slot in 0..VIRTIO_MMIO_SLOTS {
            irq::register(
                VIRTIO_NET_SPI + slot as u32,
                virtio_mmio::on_irq,
                (VIRTIO_NET_BASE + slot * VIRTIO_MMIO_STRIDE) as u32,
            )
            .map_err(BindError::HandlerNotRegistered)?;
        }
        timer::init(timer_hz).map_err(BindError::Timer)?;
        irq::enable(TIMER_IRQ);
        irq::enable(UART_IRQ);
        for slot in 0..VIRTIO_MMIO_SLOTS {
            irq::enable(VIRTIO_NET_SPI + slot as u32);
        }
        Ok(())
    }
}

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

pub fn send_resched_sgi() -> bool {
    GIC.send_sgi_raw(WAKE_SGI, CORE1_TARGET_BIT)
}

fn on_wake_sgi(_cookie: irq::IrqCookie) {
    if cpu::affinity() == 1 {
        smp::note_core1_ipi();
        smp::request_resched(1);
    }
}

/// Bring up GICC/PPI state on the secondary before entering shared policy.
///
/// # Safety
/// Called only on affinity 1 with IRQs masked and the distributor ready.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn harbor_secondary_idle() -> ! {
    while !smp::secondary_may_irq() {
        cpu::wait_for_event();
    }
    GIC.init_this_cpu();
    irq::enable(WAKE_SGI);
    let _ = timer::init_secondary();
    irq::enable(TIMER_IRQ);
    smp::mark_secondary_irq_ready();
    cpu::sync_pipeline();
    cpu::irq_enable();
    cpu::sync_pipeline();
    unsafe extern "C" {
        fn harbor_secondary_sched() -> !;
    }
    // SAFETY: the scheduler entry is linked by the product path.
    unsafe { harbor_secondary_sched() }
}

#[cfg(feature = "bringup")]
pub fn debug_peek_pending() -> Option<u32> {
    GIC.debug_hppir_id()
}

#[cfg(feature = "bringup")]
pub fn debug_read_iar() -> u32 {
    GIC.debug_iar()
}

#[cfg(feature = "bringup")]
pub fn debug_write_eoir(val: u32) {
    GIC.debug_eoir(val);
}
