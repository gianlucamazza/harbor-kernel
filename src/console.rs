//! Kernel console policy.
//!
//! Until a resident console agent exists, one owner holds an explicit [`Pl011`]
//! taken from [`acquire`] for TX; a second [`acquire`] returns `None`. RX bytes arrive via UART IRQ into a kernel
//! ring ([`on_uart_rx_irq`] → [`pop_rx`]); the IRQ path never transmits.
//!
//! Formatting helpers take the TX handle as the first argument so ownership
//! stays visible and testable.

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use kernel_core::ring::ByteRing;

use crate::arch::cpu;
use crate::bsp::board;
use crate::drivers::pl011::{Pl011, Pl011Rx};
use crate::sync::SyncCell;

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

/// Live TX handle after [`install_tx`], shared by cooperative tasks under
/// [`with_tx`]. Idle and worker tasks serialize here (ADR-0006): one core, IRQ
/// masked across the write, no second `Pl011` in the wild.
static TX: SyncCell<Option<Pl011>> = SyncCell::new(None);

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

/// Install the exclusive TX handle for [`with_tx`] / [`kprintln`].
///
/// Call once after bring-up printing is done, before spawning tasks. Moves the
/// handle into kernel storage so idle and workers share one path.
pub fn install_tx(uart: Pl011) {
    cpu::without_irqs(|| {
        // SAFETY: single core; IRQs masked; install is once after acquire.
        unsafe {
            *TX.get() = Some(uart);
        }
    });
}

/// Run `f` with the installed TX handle, IRQs masked for the duration.
///
/// `None` if [`install_tx`] has not run (or after a panic [`steal`]).
pub fn with_tx<R>(f: impl FnOnce(&mut Pl011) -> R) -> Option<R> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; only cooperative tasks call this on one core.
        unsafe { (*TX.get()).as_mut().map(f) }
    })
}

/// Write a line on the installed console (cooperative multi-task entry point).
pub fn kprint_fmt(args: core::fmt::Arguments<'_>) {
    let _ = with_tx(|uart| uart.write_fmt(args));
}

/// Take the console away from its current owner.
///
/// # Safety
///
/// The previous owner must be about to stop running. This exists for the panic
/// path, where an interleaved diagnostic beats a diagnostic nobody can print.
pub unsafe fn steal() -> Pl011 {
    CLAIMED.store(true, Ordering::Release);
    // Drop the shared handle first so `with_tx` cannot race the panic writer.
    cpu::without_irqs(|| {
        // SAFETY: panic path; cooperative tasks are not running.
        unsafe {
            *TX.get() = None;
        }
    });
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
    RX_MMIO_BASE.store(uart.receiver().base(), Ordering::Release);
    uart.enable_rx_interrupt();
}

/// IRQ handler for the platform UART RX line (BSP supplies the GIC id).
///
/// Drains the PL011 RX FIFO into the kernel ring. Must not transmit or format.
/// Cookie is unused (ADR-0008 shape).
pub fn on_uart_rx_irq(_cookie: u32) {
    let base = RX_MMIO_BASE.load(Ordering::Acquire);
    if base == 0 {
        return;
    }
    // SAFETY: base was published by `enable_rx_irq` from the live console.
    // `Pl011Rx` can only drain and acknowledge receive — it has no transmit
    // path — so this cannot violate the "IRQ handlers do not transmit" rule
    // even by mistake.
    let rx = unsafe { Pl011Rx::from_base(base) };
    let mut dropped = 0u32;
    rx.drain(|b| {
        let stored = RX_RING.push(b);
        if !stored {
            dropped += 1;
        }
        // Keep draining either way: a level-triggered RX line that is never
        // emptied re-fires forever.
        stored
    });
    if dropped != 0 {
        RX_DROPPED.fetch_add(dropped, Ordering::Relaxed);
    }
}

/// Bytes the RX IRQ had to discard because the ring was full.
///
/// The handler cannot report this itself — it is forbidden from transmitting,
/// and formatting in IRQ context is exactly the rule that keeps the console
/// usable. So it counts, and the console loop tells the story later.
///
/// A dropped byte with no counter is indistinguishable from a byte the user
/// never typed. That is the same defect as an allocator that silently accepts
/// a double free: the failure is invisible precisely when it matters, under
/// load.
static RX_DROPPED: AtomicU32 = AtomicU32::new(0);

/// How many received bytes have been discarded for want of ring space.
pub fn rx_dropped() -> u32 {
    RX_DROPPED.load(Ordering::Relaxed)
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

/// Line on the installed shared console ([`console::install_tx`]).
///
/// For cooperative tasks that must not hold a `Pl011` across a yield.
#[macro_export]
macro_rules! kprintln {
    () => {{
        $crate::console::kprint_fmt(format_args!("\n"));
    }};
    ($($arg:tt)*) => {{
        $crate::console::kprint_fmt(format_args!("{}\n", format_args!($($arg)*)));
    }};
}
