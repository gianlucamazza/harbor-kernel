//! Kernel IRQ subsystem: single chip owner + dispatch table.
//!
//! Exception entry calls [`handle_cpu_irq`]. Drivers and BSP register handlers;
//! the exception layer does not know about GIC or timer IDs.
//!
//! Interrupts that arrive without a handler are still acknowledged — the
//! alternative is a level-triggered line re-firing forever — but they are
//! counted. A silently EOI-ed interrupt storm looks exactly like an idle
//! system from the console, which is the worst way to lose an afternoon.
//!
//! # Sealing
//!
//! The dispatch table is mutable during bring-up and frozen afterwards. That
//! is what makes the IRQ path's shared read sound: after [`seal`] there is no
//! writer left to race with, so it is an invariant the code enforces rather
//! than a rule the reader has to keep. [`register`] after sealing fails.
//!
//! # Cookies (ADR-0008)
//!
//! Handlers are [`Handler`] = `fn(IrqCookie)`. The cookie is assigned at
//! registration and is what a future capability names — not a raw GIC id.

// Audit debt (2026-08-06): 3 unsafe blocks here predate
// `clippy::undocumented_unsafe_blocks` and do not yet say what makes them sound.
// This comes off when the audit reaches this module and the SAFETY comments can
// state something checkable rather than restate the code. See Cargo.toml.
#![allow(clippy::undocumented_unsafe_blocks)]

mod chip;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub use chip::{Ack, IrqChip};

use crate::arch::cpu;
use crate::sync::SyncCell;

/// Maximum interrupt id we dispatch (GICv2 SPI range fits comfortably).
const MAX_IRQ: usize = 256;

/// Interrupts claimed in one exception entry before giving the CPU back.
///
/// A bound, not a policy: without it a misconfigured level-triggered line
/// would spin here forever. Hitting it is recorded.
const MAX_CLAIMS_PER_ENTRY: u32 = 64;

/// Opaque cookie passed to every handler (ADR-0008). Not a GIC id.
pub type IrqCookie = u32;

/// Registered IRQ handler. Always takes a cookie (may be ignored).
pub type Handler = fn(IrqCookie);

struct Slot {
    handler: Handler,
    cookie: IrqCookie,
}

struct IrqState {
    chip: Option<&'static dyn IrqChip>,
    slots: [Option<Slot>; MAX_IRQ],
}

/// Chip + dispatch table.
///
/// Written only during bootstrap with IRQs masked; read from the IRQ path.
static STATE: SyncCell<IrqState> = SyncCell::new(IrqState {
    chip: None,
    slots: [const { None }; MAX_IRQ],
});

/// Set once bring-up finishes; the dispatch table is read-only from then on.
static SEALED: AtomicBool = AtomicBool::new(false);

/// Interrupts claimed with no registered handler.
static UNHANDLED: AtomicU32 = AtomicU32::new(0);
/// Interrupts claimed with an id beyond [`MAX_IRQ`].
static OUT_OF_RANGE: AtomicU32 = AtomicU32::new(0);
/// Times an exception entry hit [`MAX_CLAIMS_PER_ENTRY`] with work still pending.
static LOOP_EXHAUSTED: AtomicU32 = AtomicU32::new(0);

/// Diagnostic counters. All are expected to stay zero on a healthy boot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub unhandled: u32,
    pub out_of_range: u32,
    pub loop_exhausted: u32,
}

/// Snapshot of the dispatch counters.
pub fn counters() -> Counters {
    Counters {
        unhandled: UNHANDLED.load(Ordering::Relaxed),
        out_of_range: OUT_OF_RANGE.load(Ordering::Relaxed),
        loop_exhausted: LOOP_EXHAUSTED.load(Ordering::Relaxed),
    }
}

/// Read-only view of the state, for the IRQ path and the safe accessors.
///
/// # Safety
/// Caller must not hold a `&mut` to `STATE` at the same time. Before [`seal`]
/// that is guaranteed by every mutator running inside `cpu::without_irqs`;
/// after it, by there being no mutator at all.
#[inline]
unsafe fn state() -> &'static IrqState {
    unsafe { &*STATE.get() }
}

/// Freeze the dispatch table. Call once bring-up has registered everything.
///
/// After this the IRQ path is the only reader of immutable state, so no
/// discipline is required of anyone to keep it sound.
pub fn seal() {
    SEALED.store(true, Ordering::Release);
}

/// Install the platform irqchip. Call once before [`register`] / [`enable`].
///
/// # Safety
/// Single active core; no concurrent `handle_cpu_irq` until init completes.
pub unsafe fn init(chip: &'static dyn IrqChip) {
    unsafe {
        cpu::without_irqs(|| {
            (*STATE.get()).chip = Some(chip);
        });
        chip.init();
    }
}

/// Register a handler for `irq` with an opaque [`IrqCookie`].
///
/// Returns `false` if `irq` is beyond the dispatch table, or if the table has
/// already been sealed. Overwrites any previous handler.
///
/// # Safety
/// Call only while IRQs that use this id are masked or not yet enabled.
#[must_use = "an unregistered handler means the line will be EOI-ed and dropped"]
pub unsafe fn register(irq: u32, handler: Handler, cookie: IrqCookie) -> bool {
    unsafe {
        let id = irq as usize;
        if id >= MAX_IRQ || SEALED.load(Ordering::Acquire) {
            return false;
        }
        cpu::without_irqs(|| {
            (*STATE.get()).slots[id] = Some(Slot { handler, cookie });
        });
        true
    }
}

/// Enable `irq` on the platform chip. No-op if no chip is installed.
pub fn enable(irq: u32) {
    // SAFETY: shared read; mutation only happens with IRQs masked.
    if let Some(chip) = unsafe { state() }.chip {
        chip.enable(irq);
    }
}

/// Disable `irq` on the platform chip. No-op if no chip is installed.
///
/// Only the bring-up gates mask a line by hand today; the production path
/// enables its interrupts once and leaves them enabled.
#[cfg(feature = "bringup")]
pub fn disable(irq: u32) {
    // SAFETY: shared read; mutation only happens with IRQs masked.
    if let Some(chip) = unsafe { state() }.chip {
        chip.disable(irq);
    }
}

/// CPU IRQ exception entry: claim → dispatch → EOI loop.
///
/// Called from the vector stub with DAIF masked; does not re-enable IRQs.
pub fn handle_cpu_irq() {
    let _ = handle_cpu_irq_counted();
}

/// Same as [`handle_cpu_irq`], returns how many interrupts were claimed.
pub fn handle_cpu_irq_counted() -> u32 {
    // SAFETY: shared read; the table is only mutated with IRQs masked.
    let state = unsafe { state() };
    let Some(chip) = state.chip else {
        return 0;
    };

    let mut claimed = 0u32;

    while claimed < MAX_CLAIMS_PER_ENTRY {
        let Some(ack) = chip.claim() else {
            return claimed;
        };
        claimed += 1;

        let id = ack.interrupt_id() as usize;
        match state.slots.get(id) {
            Some(Some(slot)) => (slot.handler)(slot.cookie),
            Some(None) => {
                UNHANDLED.fetch_add(1, Ordering::Relaxed);
            }
            None => {
                OUT_OF_RANGE.fetch_add(1, Ordering::Relaxed);
            }
        }

        chip.end(ack);
    }

    // Left the loop with the budget spent rather than with nothing to claim.
    LOOP_EXHAUSTED.fetch_add(1, Ordering::Relaxed);
    claimed
}
