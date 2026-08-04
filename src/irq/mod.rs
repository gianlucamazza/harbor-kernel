//! Kernel IRQ subsystem: single chip owner + dispatch table.
//!
//! Exception entry calls [`handle_cpu_irq`]. Drivers and BSP register handlers;
//! the exception layer does not know about GIC or timer IDs.

mod chip;

pub use chip::{Ack, IrqChip};

/// Maximum interrupt id we dispatch (GICv2 SPI range fits comfortably).
const MAX_IRQ: usize = 256;

type Handler = fn();

struct IrqState {
    chip: Option<&'static dyn IrqChip>,
    handlers: [Option<Handler>; MAX_IRQ],
}

// Single-core M1: exclusive access after bootstrap init.
static mut STATE: IrqState = IrqState {
    chip: None,
    handlers: [None; MAX_IRQ],
};

/// Install the platform irqchip. Call once before [`register`] / [`enable`].
///
/// # Safety
/// Single active core; no concurrent `handle_cpu_irq` until init completes.
pub unsafe fn init(chip: &'static dyn IrqChip) {
    STATE.chip = Some(chip);
    chip.init();
}

/// Register a handler for `irq`. Overwrites any previous handler.
///
/// # Safety
/// Call only while IRQs that use this id are masked or not yet enabled.
pub unsafe fn register(irq: u32, handler: Handler) {
    let id = irq as usize;
    assert!(id < MAX_IRQ, "irq id out of range: {irq}");
    STATE.handlers[id] = Some(handler);
}

/// Enable `irq` on the platform chip.
pub fn enable(irq: u32) {
    // SAFETY: chip installed in bootstrap before any enable.
    let chip = unsafe { STATE.chip.expect("irq::init not called") };
    chip.enable(irq);
}

/// Disable `irq` on the platform chip.
pub fn disable(irq: u32) {
    let chip = unsafe { STATE.chip.expect("irq::init not called") };
    chip.disable(irq);
}

/// Highest pending id (no claim). For bring-up gates only.
pub fn peek_pending() -> Option<u32> {
    let chip = unsafe { STATE.chip.expect("irq::init not called") };
    chip.peek_pending()
}

/// CPU IRQ exception entry: claim → dispatch → EOI loop.
///
/// Called from the vector stub with DAIF masked; does not re-enable IRQs.
pub fn handle_cpu_irq() {
    let _ = handle_cpu_irq_counted();
}

/// Same as [`handle_cpu_irq`], returns how many interrupts were claimed.
pub fn handle_cpu_irq_counted() -> u32 {
    // SAFETY: single-core; chip installed before unmask.
    let chip = unsafe {
        match STATE.chip {
            Some(c) => c,
            None => return 0,
        }
    };

    let mut claimed = 0u32;

    for _ in 0..64 {
        let Some(ack) = chip.claim() else {
            break;
        };
        claimed += 1;
        let id = ack.interrupt_id() as usize;
        if id < MAX_IRQ {
            // SAFETY: handlers table only mutated while this IRQ is masked path.
            if let Some(handler) = unsafe { STATE.handlers[id] } {
                handler();
            }
        }
        chip.end(ack);
    }

    claimed
}

/// Claim a single interrupt and return its id (after running handler + EOI).
///
/// For bring-up diagnostics only.
#[allow(dead_code)]
pub fn claim_one_id() -> Option<u32> {
    let chip = unsafe { STATE.chip? };
    let ack = chip.claim()?;
    let id = ack.interrupt_id();
    let idx = id as usize;
    if idx < MAX_IRQ {
        if let Some(handler) = unsafe { STATE.handlers[idx] } {
            handler();
        }
    }
    chip.end(ack);
    Some(id)
}
