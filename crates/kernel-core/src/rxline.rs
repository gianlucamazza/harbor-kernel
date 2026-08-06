//! Who owns the UART receive line, and the order in which it changes hands.
//!
//! The kernel drains `DR` from an interrupt handler; an EL0 agent that owns the
//! PL011 polls `DR` itself ([ADR-0013](../../docs/adr/0013-narrow-device-windows.md)).
//! Both cannot be true at once, so the line is handed over — and the handover
//! is the dangerous part.
//!
//! # The state that must never exist
//!
//! UART0 is a **level-triggered** SPI. If the RX interrupt is armed while the
//! handler has no MMIO view to drain through, an arriving byte enters the
//! handler, finds nothing to do, and returns without popping `DR` or writing
//! `ICR`. The line is still asserted, so it re-presents immediately: it burns
//! the per-entry claim budget and re-enters, forever. Nothing clears it, and
//! from the console the machine looks idle.
//!
//! That is one state — *armed, no view* — and every rule in this module exists
//! to keep the line out of it:
//!
//! - suspending masks **before** it clears the view;
//! - resuming publishes the view **before** it arms.
//!
//! Both orders were once the other way round. The defect was found by reading,
//! not by any gate: the window is a couple of instructions wide and needs a byte
//! to arrive inside it, and the QEMU boot check types nothing. It was later
//! exercised on hardware by streaming a byte every 2 ms across the handover
//! (`verification.md`), which is evidence and not proof — the host tests below
//! are what state the invariant.
//!
//! # Decide, don't act
//!
//! Like [`crate::ipc::Table`] and [`crate::tasks::Tasks`], this type returns
//! what to do and never does it. The steps touch `IMSC` and `ICR` on real
//! hardware; here they are values, which is what lets a test walk the line
//! through every intermediate state and ask, after each one, whether an
//! interrupt arriving at that moment could still be cleared.

/// One hardware action in a handover, in the order the plan lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Mask RX/RT in `IMSC`, drop whatever is in the FIFO, and write `ICR`.
    /// After this the line cannot re-present, whatever the view says.
    MaskAndAck,
    /// Stop publishing an MMIO base: the handler will return immediately.
    ClearView,
    /// Publish an MMIO base the handler may drain through.
    PublishView(usize),
    /// Unmask RX/RT in `IMSC`.
    Arm,
}

/// The receive line's ownership state.
///
/// `armed` is `IMSC`; `view` is the MMIO base the handler is allowed to use,
/// and zero means it has none. The kernel mirrors `view` into an atomic the
/// interrupt handler can read without taking the mask — the same shape
/// [`crate::ipc`]'s refusal counters use, and for the same reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RxLine {
    armed: bool,
    view: usize,
}

impl Default for RxLine {
    fn default() -> Self {
        Self::new()
    }
}

impl RxLine {
    /// Before the interrupt is ever enabled: masked, and no view.
    pub const fn new() -> Self {
        Self {
            armed: false,
            view: 0,
        }
    }

    /// The state this module exists to make unreachable: an armed,
    /// level-triggered line the handler cannot clear.
    ///
    /// Public because a test that cannot ask this question can only check that
    /// a list of steps equals a list of steps.
    pub const fn is_unclearable(&self) -> bool {
        self.armed && self.view == 0
    }

    // There is deliberately no `owner()` returning a three-way enum. The first
    // draft had one, with an `Unarmed` variant for "before `enable_rx_irq`" —
    // and the state cannot express it: before an install and after a suspend
    // are both `armed: false, view: 0`, byte for byte. The variant was
    // unreachable and the two remaining arms were identical, so the function
    // said less than `may_drain` while looking like it said more.

    /// The base the interrupt handler may drain through, if any.
    ///
    /// The handler calls this first and returns on `None`. That check is what
    /// makes *masked, no view* survivable at all: the storm comes from being
    /// armed with no view, not from having no view.
    pub const fn may_drain(&self) -> Option<usize> {
        if self.view == 0 {
            None
        } else {
            Some(self.view)
        }
    }

    /// Apply one step. The caller performs the corresponding hardware action.
    pub const fn apply(&mut self, step: Step) {
        match step {
            Step::MaskAndAck => self.armed = false,
            Step::ClearView => self.view = 0,
            Step::PublishView(base) => self.view = base,
            Step::Arm => self.armed = true,
        }
    }

    /// Arm the kernel drain for the first time (`enable_rx_irq`).
    ///
    /// View before arm, for the same reason as [`Self::plan_resume`].
    pub const fn plan_install(&self, base: usize) -> Option<[Step; 2]> {
        if base == 0 {
            return None;
        }
        Some([Step::PublishView(base), Step::Arm])
    }

    /// Hand the line to an agent. `None` when the kernel does not hold it.
    ///
    /// Returns the base to give back later, and the two steps in the only order
    /// that is safe: masking first means that between the steps the line is
    /// masked-with-a-view, which is harmless, instead of
    /// armed-without-a-view, which is the storm.
    pub const fn plan_suspend(&self) -> Option<(usize, [Step; 2])> {
        if self.view == 0 {
            return None;
        }
        Some((self.view, [Step::MaskAndAck, Step::ClearView]))
    }

    /// Take the line back. `None` for a zero base, which would publish "no
    /// view" and then arm — the storm, built deliberately.
    pub const fn plan_resume(&self, base: usize) -> Option<[Step; 2]> {
        if base == 0 {
            return None;
        }
        Some([Step::PublishView(base), Step::Arm])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Walk a plan, asserting after **every** step that a byte arriving at that
    /// instant could still be cleared.
    ///
    /// This is the whole point of the module. Asserting that a plan equals a
    /// two-element array tests that a constant is a constant; asserting that no
    /// intermediate state is unclearable is the invariant the hardware cares
    /// about, and it is what goes red when the two steps are swapped.
    fn walk(line: &mut RxLine, steps: &[Step]) {
        assert!(
            !line.is_unclearable(),
            "unclearable before the plan started"
        );
        for (i, &step) in steps.iter().enumerate() {
            line.apply(step);
            assert!(
                !line.is_unclearable(),
                "step {i} ({step:?}) left the line armed with no view"
            );
        }
    }

    fn kernel_owned(base: usize) -> RxLine {
        let mut line = RxLine::new();
        let plan = line.plan_install(base).expect("non-zero base");
        walk(&mut line, &plan);
        line
    }

    #[test]
    fn a_handover_never_passes_through_the_unclearable_state() {
        let mut line = kernel_owned(0xFE20_1000);
        assert_eq!(line.may_drain(), Some(0xFE20_1000));

        let (base, steps) = line.plan_suspend().expect("the kernel holds it");
        assert_eq!(base, 0xFE20_1000);
        walk(&mut line, &steps);
        assert_eq!(line.may_drain(), None, "the handler must return at once");

        let plan = line.plan_resume(base).expect("non-zero base");
        walk(&mut line, &plan);
        assert_eq!(line.may_drain(), Some(0xFE20_1000), "and back again");
    }

    #[test]
    fn suspending_in_the_wrong_order_builds_the_storm() {
        // The defect as it was written: the view is disarmed before `IMSC` is
        // masked. This is the test that fails if someone puts it back, and it
        // fails at the exact step, not at the end.
        let mut line = kernel_owned(0xFE20_1000);
        line.apply(Step::ClearView);
        assert!(
            line.is_unclearable(),
            "armed with no view — a byte here re-presents forever"
        );
    }

    #[test]
    fn resuming_in_the_wrong_order_builds_the_same_storm() {
        // The mirror image, which the original code also had.
        let mut line = kernel_owned(0xFE20_1000);
        let (base, steps) = line.plan_suspend().unwrap();
        walk(&mut line, &steps);

        line.apply(Step::Arm);
        assert!(line.is_unclearable(), "armed before the view was published");

        line.apply(Step::PublishView(base));
        assert!(!line.is_unclearable(), "and cleared again once it is");
    }

    #[test]
    fn a_second_suspend_is_refused_rather_than_returning_zero_as_a_base() {
        // Two agents, or one agent twice: the second call must not report a
        // base of zero as if it were one, because the caller would later
        // `plan_resume(0)` and publish "no view" before arming.
        let mut line = kernel_owned(0xFE20_1000);
        let (_, steps) = line.plan_suspend().unwrap();
        walk(&mut line, &steps);

        assert_eq!(line.plan_suspend(), None, "nothing left to hand over");
    }

    #[test]
    fn resuming_with_a_zero_base_is_refused() {
        // `plan_resume(0)` would be `PublishView(0)` then `Arm` — the storm,
        // assembled out of two individually reasonable steps.
        let mut line = kernel_owned(0xFE20_1000);
        let (_, steps) = line.plan_suspend().unwrap();
        walk(&mut line, &steps);

        assert_eq!(line.plan_resume(0), None);
        assert!(!line.is_unclearable(), "and the line is untouched");
    }

    #[test]
    fn installing_a_zero_base_is_refused() {
        // Same shape at the other end: `enable_rx_irq` with no MMIO base.
        let line = RxLine::new();
        assert_eq!(line.plan_install(0), None);
    }

    #[test]
    fn the_handler_may_drain_exactly_when_the_kernel_holds_the_line() {
        let mut line = RxLine::new();
        assert_eq!(line.may_drain(), None, "before install");

        let plan = line.plan_install(0x1000).unwrap();
        walk(&mut line, &plan);
        assert_eq!(line.may_drain(), Some(0x1000));

        let (base, steps) = line.plan_suspend().unwrap();
        walk(&mut line, &steps);
        assert_eq!(line.may_drain(), None, "while the agent owns it");

        let plan = line.plan_resume(base).unwrap();
        walk(&mut line, &plan);
        assert_eq!(line.may_drain(), Some(0x1000), "and back again");
    }

    #[test]
    fn a_masked_line_with_a_view_is_not_a_storm() {
        // The intermediate state a correct suspend passes through. It is not
        // dangerous, and calling it dangerous would make the safe order look
        // as bad as the unsafe one.
        let mut line = kernel_owned(0x2000);
        line.apply(Step::MaskAndAck);
        assert!(!line.is_unclearable());
        assert_eq!(line.may_drain(), Some(0x2000));
    }
}
