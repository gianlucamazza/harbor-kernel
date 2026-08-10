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
//! # What is here and what is not
//!
//! The table itself — which handler owns a line, the bounds, the seal, and the
//! three-way answer for a claimed interrupt — is
//! [`kernel_core::irqtable::Table`], where it is host-tested. It had to move:
//! the safety argument above rests on registration failing after the seal, and
//! nothing had ever registered a handler after sealing to watch it fail.
//!
//! What stays here is what a pure table cannot do: own the chip, take the
//! interrupt mask, publish the counters an exception context can reach, and
//! call the handler.
//!
//! # Cookies (ADR-0008)
//!
//! Handlers are [`Handler`] = `fn(IrqCookie)`. The cookie is assigned at
//! registration and is what a future capability names — not a raw GIC id.

pub mod cap;
mod chip;
pub mod wait;

use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::irqtable::{Dispatch, Table};

pub use chip::{Ack, IrqChip};
pub use kernel_core::irqtable::RegisterError;

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

struct IrqState {
    chip: Option<&'static dyn IrqChip>,
    table: Table<Handler, MAX_IRQ>,
}

/// Chip + dispatch table.
///
/// Written only during bootstrap with IRQs masked; read from the IRQ path.
static STATE: SyncCell<IrqState> = SyncCell::new(IrqState {
    chip: None,
    table: Table::new(),
});

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

/// Run `f` over a shared view of the state; the borrow ends with the closure.
///
/// This used to be `unsafe fn state() -> &'static IrqState` — an unbounded
/// shared borrow a caller could legally hold across a `register`, with only
/// prose stopping it (excellence review F-26). The closure scope makes that
/// unwritable: before [`seal`], every mutator runs inside `cpu::without_irqs`;
/// after it, there is no mutator at all, and either way no reference outlives
/// `f`.
#[inline]
fn with_state<R>(f: impl FnOnce(&IrqState) -> R) -> R {
    // SAFETY: shared borrow scoped to `f`, immutability per the note above.
    f(unsafe { &*STATE.get() })
}

/// Freeze the dispatch table. Call once bring-up has registered everything.
///
/// After this the IRQ path is the only reader of immutable state, so no
/// discipline is required of anyone to keep it sound.
pub fn seal() {
    // One flag, in the table. An earlier draft kept a separate `AtomicBool`
    // beside it so a reader outside the interrupt mask could observe the seal —
    // and nothing ever read it. Two sources of truth for one fact, one of them
    // dead, is how they drift; the compiler said so and the atomic is gone.
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked and one core, so this `&mut` cannot overlap the
        // IRQ path's shared borrow.
        unsafe { (*STATE.get()).table.seal() };
    });
}

/// How many lines have a handler. Reported at bring-up, so a boot that
/// registered nothing is visible before the first interrupt rather than after.
pub fn registered() -> usize {
    with_state(|s| s.table.registered())
}

/// Install the platform irqchip. Call once before [`register`] / [`enable`].
///
/// # Safety
/// Single active core; no concurrent `handle_cpu_irq` until init completes.
pub unsafe fn init(chip: &'static dyn IrqChip) {
    // SAFETY: the caller guarantees no `handle_cpu_irq` can run until this
    // returns, and the write itself is inside `without_irqs` so it cannot be
    // observed half-done by this core either. `chip.init()` follows the store
    // so the dispatch table already names the chip it is about to program.
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
pub unsafe fn register(irq: u32, handler: Handler, cookie: IrqCookie) -> Result<(), RegisterError> {
    // SAFETY: the caller guarantees this line is masked or not yet enabled, so
    // no handler can be dispatched for `irq` while its slot is being written,
    // and the write happens inside `without_irqs` so it cannot overlap the IRQ
    // path's shared borrow either. The table refuses after sealing, which is
    // what keeps this from mutating state a live IRQ path is reading.
    //
    // `cookie` is stored and never read. Both handlers take it and ignore it
    // (`fn on_timer_irq(_cookie: u32)`); ADR-0008 specifies the shape, and the
    // consumer it was specified for — per-line context for a driver agent —
    // does not exist yet.
    unsafe { cpu::without_irqs(|| (*STATE.get()).table.register(irq, handler, cookie)) }
}

/// Enable `irq` on the platform chip. No-op if no chip is installed.
pub fn enable(irq: u32) {
    if let Some(chip) = with_state(|s| s.chip) {
        chip.enable(irq);
    }
}

/// Raise an SGI on the named CPU target list (ADR-0074/0076). No-op if no chip.
#[inline]
pub fn send_sgi(sgi_id: u32, cpu_target_list: u8) -> bool {
    with_state(|s| s.chip.map(|c| c.send_sgi(sgi_id, cpu_target_list)).unwrap_or(false))
}

/// Disable `irq` on the platform chip. No-op if no chip is installed.
///
/// Only the bring-up gates mask a line by hand today; the production path
/// enables its interrupts once and leaves them enabled.
#[cfg(feature = "bringup")]
pub fn disable(irq: u32) {
    if let Some(chip) = with_state(|s| s.chip) {
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
    // The whole claim/dispatch loop runs under one scoped shared borrow: the
    // table is sealed before interrupts enable, so handlers dispatched inside
    // it cannot mutate the state they were dispatched from.
    with_state(|state| {
        let Some(chip) = state.chip else {
            return 0;
        };

        let mut claimed = 0u32;

        while claimed < MAX_CLAIMS_PER_ENTRY {
            let Some(ack) = chip.claim() else {
                return claimed;
            };
            claimed += 1;

            match state.table.lookup(ack.interrupt_id()) {
                Dispatch::Handle { handler, cookie } => handler(cookie),
                Dispatch::Unhandled => {
                    UNHANDLED.fetch_add(1, Ordering::Relaxed);
                }
                Dispatch::OutOfRange => {
                    OUT_OF_RANGE.fetch_add(1, Ordering::Relaxed);
                }
            }

            chip.end(ack);
        }

        // Left the loop with the budget spent rather than nothing to claim.
        LOOP_EXHAUSTED.fetch_add(1, Ordering::Relaxed);
        claimed
    })
}
