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
use kernel_core::rxline::{RxLine, Step};

use crate::arch::cpu;
use crate::bsp::board;
use crate::drivers::pl011::{Pl011, Pl011Rx};
use crate::sync::SyncCell;

/// RX ring capacity (usable = CAP − 1). Power of two.
const RX_CAP: usize = 256;

/// IRQ producer → main-loop consumer. `ByteRing` takes `&self`, so the two
/// contexts share this `static` without ever forming aliasing `&mut`.
static RX_RING: ByteRing<RX_CAP> = ByteRing::new();

/// Who owns the receive line, and in what order that may change.
///
/// The rules live in [`kernel_core::rxline`], where they are host-tested: a
/// level-triggered line that is armed while the handler has no view to drain
/// through cannot be cleared, and both handover orders once had that state in
/// the middle of them. This module executes the steps it is given; it does not
/// decide them.
///
/// Written only with IRQs masked, which is what makes the plain `&mut` sound.
static RX_LINE: SyncCell<RxLine> = SyncCell::new(RxLine::new());

/// MMIO base the RX IRQ drains from; 0 until [`enable_rx_irq`] arms it.
///
/// A publication of [`RX_LINE`]'s view for the one reader that cannot take the
/// interrupt mask — the handler itself. The same shape `src/ipc` uses for the
/// refusal counters, and for the same reason.
///
/// An `Option<Mmio>` here would be written by the main loop and read by the
/// IRQ with no atomicity: the handler could observe it half-initialised.
static RX_MMIO_BASE: AtomicUsize = AtomicUsize::new(0);

/// Run `f` against the line with IRQs masked.
fn with_line<R>(f: impl FnOnce(&mut RxLine) -> R) -> R {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked and one core, so this `&mut` cannot overlap
        // another. The IRQ handler never touches `RX_LINE` — it reads the
        // published atomic instead, which is why that publication exists.
        let line = unsafe { &mut *RX_LINE.get() };
        f(line)
    })
}

/// Perform one step and record it. The hardware action and the model move
/// together, so the model cannot describe a line the hardware is not in.
///
/// Returns `false` if the TX handle is gone and a masking step could not be
/// applied — the caller must then abandon the plan rather than continue with a
/// half-applied one.
fn apply(line: &mut RxLine, step: Step) -> bool {
    let performed = match step {
        Step::MaskAndAck => with_tx(|uart| {
            uart.disable_rx_interrupt();
            uart.receiver().discard_and_ack();
        })
        .is_some(),
        Step::Arm => with_tx(|uart| {
            uart.receiver().discard_and_ack();
            uart.enable_rx_interrupt();
        })
        .is_some(),
        Step::ClearView => {
            RX_MMIO_BASE.store(0, Ordering::Release);
            true
        }
        Step::PublishView(base) => {
            RX_MMIO_BASE.store(base, Ordering::Release);
            true
        }
    };
    if performed {
        line.apply(step);
    }
    performed
}

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
    //
    // The model is cleared with it. This is the panic path, where `IMSC` is
    // about to be reprogrammed from a cold reset anyway — so the line is not
    // left armed-without-a-view even though only the view is cleared here.
    RX_MMIO_BASE.store(0, Ordering::Release);
    with_line(|line| line.apply(Step::ClearView));
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
    let base = uart.receiver().base();
    with_line(|line| {
        let Some(steps) = line.plan_install(base) else {
            return;
        };
        // Not routed through `apply`: the caller holds the live `Pl011` and
        // `with_tx` may not be installed yet at this point in bring-up.
        for step in steps {
            match step {
                Step::PublishView(b) => RX_MMIO_BASE.store(b, Ordering::Release),
                Step::Arm => uart.enable_rx_interrupt(),
                other => unreachable!("install plans only publish and arm, got {other:?}"),
            }
            line.apply(step);
        }
    });
}

/// Pause the kernel RX drain so an EL0 agent can own `DR` (poll).
///
/// Masks PL011 RX/RT IRQs and ACKs the line so a level-triggered SPI cannot
/// storm into a no-op handler. Discards any FIFO bytes still pending for the
/// kernel ring. Returns the previous MMIO base (0 if already suspended).
///
/// While suspended, the agent maps the PL011 page and polls; the idle task
/// sees an empty ring (no echo). TX / panic paths stay kernel-owned.
///
/// Order is load-bearing: `IMSC` is masked and the line ACKed **before** the
/// IRQ view is disarmed. Disarming first leaves a window in which a byte makes
/// [`on_uart_rx_irq`] return without popping `DR` or writing `ICR` — and UART0
/// is a level-triggered SPI, so the line re-presents immediately, burns the
/// per-entry claim budget, and re-enters before this task can reach the
/// `with_tx` that would clear it.
///
/// Returns 0 without disarming anything if the TX handle is missing, since the
/// mask could not be applied: the caller sees "already suspended" rather than
/// an unclearable interrupt.
pub fn suspend_rx() -> usize {
    with_line(|line| {
        let Some((base, steps)) = line.plan_suspend() else {
            return 0;
        };
        for step in steps {
            if !apply(line, step) {
                // The mask could not be applied, so nothing after it may be
                // either: clearing the view now would leave the line armed with
                // nothing to drain through. The caller sees "already
                // suspended" and the line is exactly as it was.
                return 0;
            }
        }
        base
    })
}

/// Restore kernel RX drain after [`suspend_rx`]. No-op if `base == 0`.
///
/// Drops any leftover FIFO data from the agent window, republishes the MMIO
/// base for [`on_uart_rx_irq`], and only then re-arms RX IRQs.
///
/// Mirror of [`suspend_rx`]: the view is armed before `IMSC`, because the
/// reverse order lets a byte fire the handler while the base is still 0 —
/// which on a level-triggered line is the same unclearable storm.
pub fn resume_rx(base: usize) {
    with_line(|line| {
        let Some(steps) = line.plan_resume(base) else {
            return;
        };
        for step in steps {
            if !apply(line, step) {
                return;
            }
        }
    });
}

/// `true` when kernel RX drain is suspended (agent may own the line).
#[inline]
pub fn rx_drain_suspended() -> bool {
    RX_MMIO_BASE.load(Ordering::Acquire) == 0
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
