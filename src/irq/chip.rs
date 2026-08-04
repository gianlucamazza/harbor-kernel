//! Interrupt controller abstraction (irqchip).

/// Full acknowledge word returned by a claim (must be passed back to [`IrqChip::end`]).
#[derive(Clone, Copy, Debug)]
pub struct Ack(pub u32);

impl Ack {
    #[inline]
    pub const fn interrupt_id(self) -> u32 {
        // Low 10 bits are the interrupt id; upper bits may carry chip flags.
        self.0 & 0x3FF
    }

    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Board-agnostic interrupt controller operations.
///
/// Implemented by `drivers::gicv2::GicV2`. Owned once via [`crate::irq::init`].
pub trait IrqChip: Sync {
    /// Program the controller for non-secure IRQ delivery on this CPU.
    fn init(&self);

    /// Unmask `irq` on this CPU / distributor as appropriate.
    fn enable(&self, irq: u32);

    /// Mask `irq`.
    fn disable(&self, irq: u32);

    /// Claim the highest pending interrupt, or `None` if none/spurious.
    fn claim(&self) -> Option<Ack>;

    /// Complete a previously claimed interrupt.
    fn end(&self, ack: Ack);

    /// Highest pending id without claiming (diagnostics / bring-up gates).
    fn peek_pending(&self) -> Option<u32>;
}
