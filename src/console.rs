//! Kernel console policy.
//!
//! Until a resident console agent exists, callers hold an explicit [`Pl011`]
//! obtained via [`acquire`] for TX. RX bytes arrive via UART IRQ into a kernel
//! ring ([`on_uart_rx_irq`] → [`pop_rx`]); the IRQ path never transmits.
//!
//! Formatting helpers take the TX handle as the first argument so ownership
//! stays visible and testable.

use core::ptr::addr_of_mut;

use kernel_core::ring::ByteRing;

use crate::bsp::board;
use crate::drivers::pl011::Pl011;

/// RX ring capacity (usable = CAP − 1). Power of two.
const RX_CAP: usize = 256;

// Single-core: IRQ producer writes head, main consumer writes tail.
static mut RX_RING: ByteRing<RX_CAP> = ByteRing::new();

// Second MMIO handle for the IRQ drain path (same PL011 as the TX owner).
static mut RX_MMIO: Option<crate::arch::mmio::Mmio> = None;

/// Bring up the board serial console and return exclusive TX ownership.
///
/// Each call re-programs pinmux and the UART from a known reset state. That
/// makes the operation valid both on the cold boot path and in the panic
/// handler (where any previous handle may already be invalid).
///
/// # Safety
///
/// The caller must guarantee exclusive ownership of the console hardware
/// (GPIO pinmux for the UART pins and the PL011 MMIO block) for the entire
/// lifetime of the returned handle, aside from the RX IRQ path which only
/// drains RX after [`enable_rx_irq`].
pub unsafe fn acquire() -> Pl011 {
    // Panic / re-init: drop IRQ view so we do not drain with a stale config.
    RX_MMIO = None;
    board::console::init()
}

/// Arm PL011 RX interrupts and install the IRQ-side MMIO view.
///
/// Call once after GIC has registered [`on_uart_rx_irq`], before unmasking
/// CPU IRQs. The TX handle remains with the caller.
///
/// # Safety
///
/// `uart` must be the live console PL011; exclusive TX + IRQ-only RX for the
/// rest of the boot.
pub unsafe fn enable_rx_irq(uart: &Pl011) {
    RX_MMIO = Some(uart.mmio());
    uart.enable_rx_interrupt();
}

/// IRQ handler for the platform UART RX line (BSP supplies the GIC id).
///
/// Drains the PL011 RX FIFO into the kernel ring. Must not transmit or format.
pub fn on_uart_rx_irq() {
    let Some(mmio) = (unsafe { RX_MMIO }) else {
        return;
    };
    // SAFETY: mmio set in enable_rx_irq; single-core exclusive with TX owner
    // (TX only writes; RX drain only reads DR/FR and ICR).
    let uart = unsafe { Pl011::from_mmio(mmio) };
    let ring = unsafe { &mut *addr_of_mut!(RX_RING) };
    uart.drain_rx(|b| ring.push(b));
}

/// Pop one RX byte from the IRQ-filled ring (`None` if empty).
pub fn pop_rx() -> Option<u8> {
    let ring = unsafe { &mut *addr_of_mut!(RX_RING) };
    ring.pop()
}

/// `true` when the RX ring has no bytes for the consumer.
pub fn rx_is_empty() -> bool {
    let ring = unsafe { &*core::ptr::addr_of!(RX_RING) };
    ring.is_empty()
}

/// Write formatted output to a caller-owned console.
#[macro_export]
macro_rules! print {
    ($uart:expr, $($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = core::write!($uart, $($arg)*);
    }};
}

/// Write a line of formatted output to a caller-owned console.
#[macro_export]
macro_rules! println {
    ($uart:expr) => {{
        $crate::print!($uart, "\n");
    }};
    ($uart:expr, $($arg:tt)*) => {{
        $crate::print!($uart, "{}\n", format_args!($($arg)*));
    }};
}
