//! IRQ dispatch table: which handler owns a line, and when that stops changing.
//!
//! The kernel's IRQ path reads this table from an exception context while
//! bring-up writes it, and what makes that sound is [`Table::seal`]: mutable
//! during bring-up, frozen afterwards, so after sealing there is no writer left
//! to race with. `src/irq` states that as the reason its shared `&'static`
//! borrow is sound.
//!
//! It was a rule nothing checked. `seal()` set a flag, `register()` read it,
//! and no test ever registered a handler after sealing to watch the refusal —
//! so the invariant the safety argument rests on was asserted by a comment.
//! That is what this module exists to change.
//!
//! Generic over the handler type so the crate stays free of kernel types: the
//! kernel instantiates it with its own `fn(IrqCookie)`.

/// Why a registration was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// The id is beyond the table. Not a failure of the caller's authority —
    /// the table simply does not reach that far.
    OutOfRange { irq: u32, slots: usize },
    /// Bring-up is over and the table is immutable. A handler registered here
    /// would be a write racing the IRQ path's read.
    Sealed { irq: u32 },
}

/// What the table says about an interrupt that was just claimed.
///
/// Three outcomes, deliberately distinct. An id with no handler and an id past
/// the end of the table are both "nobody handles this", and they mean very
/// different things: the first is a line someone enabled and forgot to claim,
/// the second is a chip reporting an id this kernel does not believe in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch<T> {
    /// Call this handler with this cookie.
    Handle { handler: T, cookie: u32 },
    /// In range, no handler registered.
    Unhandled,
    /// Beyond the table.
    OutOfRange,
}

#[derive(Clone, Copy)]
struct Slot<T> {
    handler: T,
    cookie: u32,
}

/// Fixed-size dispatch table with a one-way seal.
pub struct Table<T, const SLOTS: usize> {
    slots: [Option<Slot<T>>; SLOTS],
    sealed: bool,
}

impl<T: Copy, const SLOTS: usize> Default for Table<T, SLOTS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy, const SLOTS: usize> Table<T, SLOTS> {
    /// Empty and unsealed.
    pub const fn new() -> Self {
        Self {
            slots: [const { None }; SLOTS],
            sealed: false,
        }
    }

    /// Freeze the table. One-way: there is no unseal, because an unseal would
    /// give back exactly the writer that sealing exists to remove.
    pub const fn seal(&mut self) {
        self.sealed = true;
    }

    /// Whether the table is frozen.
    pub const fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Claim `irq` for `handler`. Overwrites any previous handler.
    ///
    /// The order of the two checks is not arbitrary. Sealing is tested first
    /// so that a sealed table refuses every registration alike, rather than
    /// reporting the more specific complaint about an id it would refuse
    /// anyway — a sealed table has one answer and it is the same one.
    pub const fn register(
        &mut self,
        irq: u32,
        handler: T,
        cookie: u32,
    ) -> Result<(), RegisterError> {
        if self.sealed {
            return Err(RegisterError::Sealed { irq });
        }
        let id = irq as usize;
        if id >= SLOTS {
            return Err(RegisterError::OutOfRange { irq, slots: SLOTS });
        }
        self.slots[id] = Some(Slot { handler, cookie });
        Ok(())
    }

    /// What to do with a claimed interrupt.
    pub fn lookup(&self, irq: u32) -> Dispatch<T> {
        match self.slots.get(irq as usize) {
            Some(Some(slot)) => Dispatch::Handle {
                handler: slot.handler,
                cookie: slot.cookie,
            },
            Some(None) => Dispatch::Unhandled,
            None => Dispatch::OutOfRange,
        }
    }

    /// How many lines have a handler. Bring-up reports this so a boot that
    /// silently registered nothing is visible before the first interrupt.
    pub fn registered(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is generic over the handler, so these tests use a plain number
    /// as one. That is not a shortcut — it is the fix for a test that failed
    /// intermittently.
    ///
    /// The first version used two empty `fn(u32)` items and compared them by
    /// address. Two functions with identical bodies may be folded into one
    /// symbol, so `a as usize == b as usize` sometimes held and sometimes did
    /// not, depending on how codegen units happened to be partitioned. The
    /// suite went red once and green on the next run with no change, which is
    /// worse than a failure: nothing about the table was wrong either time.
    ///
    /// Identity of the stored value is what these tests are about, and a `u32`
    /// has one. `a_real_function_pointer_fits_the_table` below keeps the
    /// instantiation the kernel actually uses covered, without asserting on an
    /// address.
    type T4 = Table<u32, 4>;

    const A: u32 = 0xAAAA;
    const B: u32 = 0xBBBB;

    fn stored(d: Dispatch<u32>) -> Option<(u32, u32)> {
        match d {
            Dispatch::Handle { handler, cookie } => Some((handler, cookie)),
            _ => None,
        }
    }

    #[test]
    fn a_registered_handler_comes_back_with_its_cookie() {
        let mut t = T4::new();
        assert_eq!(t.register(2, A, 7), Ok(()));
        assert_eq!(stored(t.lookup(2)), Some((A, 7)));
    }

    #[test]
    fn a_real_function_pointer_fits_the_table() {
        // The instantiation the kernel uses. No address comparison: this asserts
        // that a `fn(u32)` can be stored and dispatched, and calling it through
        // the table is what proves the right one came back.
        use core::cell::Cell;
        thread_local! {
            static CALLED_WITH: Cell<u32> = const { Cell::new(0) };
        }
        fn record(cookie: u32) {
            CALLED_WITH.with(|c| c.set(cookie));
        }

        let mut t: Table<fn(u32), 4> = Table::new();
        assert_eq!(t.register(1, record as fn(u32), 42), Ok(()));
        match t.lookup(1) {
            Dispatch::Handle { handler, cookie } => handler(cookie),
            other => panic!("expected a handler, got {other:?}"),
        }
        assert_eq!(CALLED_WITH.with(|c| c.get()), 42);
    }

    #[test]
    fn sealing_refuses_every_later_registration() {
        // The invariant `src/irq` rests its safety argument on, and the one
        // nothing checked before this test existed: after sealing there is no
        // writer left, so the IRQ path's shared borrow cannot race one.
        let mut t = T4::new();
        assert!(!t.is_sealed());
        assert_eq!(t.register(1, A, 0), Ok(()));

        t.seal();
        assert!(t.is_sealed());
        assert_eq!(t.register(2, A, 0), Err(RegisterError::Sealed { irq: 2 }));
        // Including over a slot that already has a handler: overwriting is the
        // same write, and the seal is about writes and not about novelty.
        assert_eq!(t.register(1, B, 0), Err(RegisterError::Sealed { irq: 1 }));
        assert_eq!(
            stored(t.lookup(1)),
            Some((A, 0)),
            "the refused registration left the old handler in place"
        );
    }

    #[test]
    fn a_sealed_table_gives_one_answer_even_for_a_bad_id() {
        // Ordering of the two checks. An out-of-range id on a sealed table is
        // refused as sealed, because a sealed table refuses everything alike;
        // reporting the range instead would suggest that fixing the id would
        // help.
        let mut t = T4::new();
        t.seal();
        assert_eq!(t.register(99, A, 0), Err(RegisterError::Sealed { irq: 99 }));
    }

    #[test]
    fn the_last_slot_is_usable_and_the_next_one_is_not() {
        // The boundary itself. `>=` and `>` differ only here.
        let mut t = T4::new();
        assert_eq!(t.register(3, A, 0), Ok(()), "slot 3 of 4");
        assert_eq!(
            t.register(4, A, 0),
            Err(RegisterError::OutOfRange { irq: 4, slots: 4 })
        );
    }

    #[test]
    fn an_unhandled_line_is_not_the_same_as_an_impossible_one() {
        // Both mean "nobody handles this" and the kernel counts them apart: an
        // in-range miss is a line somebody enabled and forgot, while an id past
        // the table is a chip reporting something this kernel does not believe
        // in. Collapsing them would hide the second inside the first.
        let t = T4::new();
        assert_eq!(t.lookup(0), Dispatch::Unhandled);
        assert_eq!(t.lookup(4), Dispatch::OutOfRange);
        assert_eq!(t.lookup(u32::MAX), Dispatch::OutOfRange);
    }

    #[test]
    fn registering_twice_replaces_rather_than_accumulates() {
        let mut t = T4::new();
        assert_eq!(t.register(0, A, 1), Ok(()));
        assert_eq!(t.register(0, B, 2), Ok(()));
        assert_eq!(stored(t.lookup(0)), Some((B, 2)));
        assert_eq!(t.registered(), 1, "one line, not two");
    }

    #[test]
    fn the_registered_count_tracks_lines_and_not_calls() {
        let mut t = T4::new();
        assert_eq!(t.registered(), 0);
        for irq in 0..4 {
            assert_eq!(t.register(irq, A, 0), Ok(()));
        }
        assert_eq!(t.registered(), 4);
        // A refused registration must not move it.
        assert!(t.register(4, A, 0).is_err());
        assert_eq!(t.registered(), 4);
    }
}
