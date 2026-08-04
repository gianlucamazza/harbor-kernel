//! Kernel console policy.
//!
//! Until a resident console agent exists, callers hold an explicit [`Pl011`]
//! obtained via [`acquire`] for TX. RX bytes arrive via UART IRQ into a kernel
//! ring ([`on_uart_rx_irq`] → [`pop_rx`]); the IRQ path never transmits.
//!
//! Formatting helpers take the TX handle as the first argument so ownership
//! stays visible and testable.

use core::sync::atomic::{AtomicUsize, Ordering};

use kernel_core::ring::ByteRing;

use crate::arch::mmio::Mmio;
use crate::bsp::board;
use crate::drivers::pl011::Pl011;

/// RX ring capacity (usable = CAP − 1). Power of two.
const RX_CAP: usize = 256;

/// IRQ producer → main-loop consumer. `ByteRing` takes `&self`, so the two
/// contexts share this `static` without ever forming aliasing `&mut`.
static RX_RING: ByteRing<RX_CAP> = ByteRing::new();

/// MMIO base the RX IRQ drains from; 0 until [`enable_rx_irq`] arms it.
///
/// An `Option<Mmio>` here would be written by the main loop and read by the
/// IRQ with no atomicity: the handler could observe it half-initialised.
static RX_MMIO_BASE: AtomicUsize = AtomicUsize::new(0);

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
    unsafe {
        // Panic / re-init: disarm the IRQ view so we do not drain with a stale
        // config. Release so the handler cannot see the disarm out of order.
        RX_MMIO_BASE.store(0, Ordering::Release);
        board::console::init()
    }
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
    RX_MMIO_BASE.store(uart.mmio().base(), Ordering::Release);
    uart.enable_rx_interrupt();
}

/// IRQ handler for the platform UART RX line (BSP supplies the GIC id).
///
/// Drains the PL011 RX FIFO into the kernel ring. Must not transmit or format.
pub fn on_uart_rx_irq() {
    let base = RX_MMIO_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    // SAFETY: base was published by `enable_rx_irq` from the live console
    // PL011. Sharing the block with the TX owner is sound because the two
    // touch disjoint behaviour: TX writes DR, this path reads DR/FR and
    // acknowledges via ICR.
    let uart = unsafe { Pl011::from_mmio(Mmio::new(base)) };
    uart.drain_rx(|b| RX_RING.push(b));
}

/// Pop one RX byte from the IRQ-filled ring (`None` if empty).
pub fn pop_rx() -> Option<u8> {
    RX_RING.pop()
}

/// `true` when the RX ring has no bytes for the consumer.
pub fn rx_is_empty() -> bool {
    RX_RING.is_empty()
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
