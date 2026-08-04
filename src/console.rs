//! Kernel console policy.
//!
//! Until a resident console agent exists, one owner holds an explicit [`Pl011`]
//! taken from [`acquire`] for TX; a second [`acquire`] returns `None`. RX bytes arrive via UART IRQ into a kernel
//! ring ([`on_uart_rx_irq`] → [`pop_rx`]); the IRQ path never transmits.
//!
//! Formatting helpers take the TX handle as the first argument so ownership
//! stays visible and testable.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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

/// Set once the console has been handed to an owner.
///
/// Enforcing the single owner here makes the panic path's override an explicit
/// [`steal`] instead of the undocumented second `acquire()` it used to be.
///
/// A real atomic, not a single-core assumption: `boot.s` enables the early
/// identity map before any Rust runs, so memory has attributes everywhere and
/// the compare-and-set behaves. This stays correct when a second core appears.
static CLAIMED: AtomicBool = AtomicBool::new(false);

/// Bring up the board serial console and take exclusive TX ownership.
///
/// `None` if the console is already owned.
///
/// # Safety
///
/// No other subsystem may be driving the UART0 pins or the PL011 register
/// block; the RX IRQ path is the one exception, and only after
/// [`enable_rx_irq`].
pub unsafe fn acquire() -> Option<Pl011> {
    if CLAIMED.swap(true, Ordering::Acquire) {
        return None;
    }
    // SAFETY: we won the claim, so no other handle exists.
    unsafe { Some(reprogram()) }
}

/// Take the console away from its current owner.
///
/// # Safety
///
/// The previous owner must be about to stop running. This exists for the panic
/// path, where an interleaved diagnostic beats a diagnostic nobody can print.
pub unsafe fn steal() -> Pl011 {
    CLAIMED.store(true, Ordering::Release);
    // SAFETY: forwarded from the caller's obligation.
    unsafe { reprogram() }
}

/// Re-program pinmux and the PL011 from a known reset state.
///
/// # Safety
/// Caller holds the exclusive claim.
unsafe fn reprogram() -> Pl011 {
    // Disarm the IRQ view so the handler cannot drain with a stale config.
    // Release so it cannot observe the disarm out of order.
    RX_MMIO_BASE.store(0, Ordering::Release);
    // SAFETY: forwarded from the caller's obligation.
    unsafe { board::console::init() }
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
