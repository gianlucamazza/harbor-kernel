//! Capability id encoding (pure arithmetic for M4).
//!
//! A [`CapId`] packs a table index and a generation so a recycled slot cannot
//! be confused with a stale handle. Rights bits travel with the endpoint table
//! entry in the kernel crate; this module only frames the id.
//!
//! ## What is unforgeable, and what is not
//!
//! Not this type. [`CapId::new`] and [`CapId::from_raw`] are `const fn` that
//! build any id from any integer, and `bootstrap`'s forger demo uses exactly
//! that to mint one. Unforgeability is a property of the *system*, not of the
//! struct: inside the kernel `ipc::lookup_endpoint` checks the id against a
//! live table entry with a matching generation, while `sched::current_holds`
//! checks that the calling task was given it. The module used to call this "the
//! unforgeable id", which reads as though the type carried the guarantee.
//!
//! **EL0 never sees a `CapId` at all** (ADR-0017 §2). An agent passes a slot
//! index into its own table and [`from_slot`] resolves it, so the strongest
//! form of the property holds exactly where it is worth its cost: an agent
//! cannot *name* another agent's capability, rather than being stopped by a
//! check when it tries. A check can have a bug; an array bound is that bug's
//! absence.
//!
//! ## The generation check has never fired
//!
//! It is the part that makes a *stale* handle detectable, as opposed to an
//! invented one, and it is correct — but nothing exercises it. `ipc::create`
//! bumps the counter per channel, and no endpoint is ever released: `live`
//! never returns to `false`, so a slot is never reused and no id can outlive
//! the entry it names. The check is a protection nobody has seen fire, which
//! this project treats as an assumption rather than a guarantee.

/// Opaque capability handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CapId(u32);

impl CapId {
    /// Index bits (low).
    pub const INDEX_BITS: u32 = 16;
    /// Generation bits (high).
    pub const GEN_BITS: u32 = 16;

    pub const INDEX_MASK: u32 = (1 << Self::INDEX_BITS) - 1;

    /// Build from table index and generation (both masked to their fields).
    ///
    /// Note what this being `pub const fn` means: anyone can mint any id.
    /// Unforgeability comes from the lookup, not from the type.
    ///
    /// ```
    /// use kernel_core::cap::CapId;
    ///
    /// let cap = CapId::new(3, 0xAB);
    /// assert_eq!(cap.index(), 3);
    /// assert_eq!(cap.generation(), 0xAB);
    ///
    /// // Same slot, different generation: a stale handle does not compare
    /// // equal to a live one, which is what the generation field is for.
    /// assert_ne!(CapId::new(3, 0xAB), CapId::new(3, 0xAC));
    /// ```
    #[inline]
    pub const fn new(index: u16, generation: u16) -> Self {
        Self(((generation as u32) << Self::INDEX_BITS) | (index as u32))
    }

    /// Table index.
    #[inline]
    pub const fn index(self) -> u16 {
        (self.0 & Self::INDEX_MASK) as u16
    }

    /// Generation counter at mint time.
    #[inline]
    pub const fn generation(self) -> u16 {
        (self.0 >> Self::INDEX_BITS) as u16
    }

    /// Raw bits (for tests / debug only).
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Reconstruct from raw bits (does not validate against a table).
    #[inline]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// Endpoint rights (bit flags).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapRights(u8);

impl CapRights {
    pub const SEND: Self = Self(1 << 0);
    pub const RECV: Self = Self(1 << 1);
    /// Wait on an IRQ notification cookie (ADR-0030). Not SEND/RECV.
    pub const IRQ: Self = Self(1 << 2);

    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Why a slot did not name a capability.
///
/// Two variants and not one, because they are two different bugs. *Out of
/// range* means the agent named something outside its own table — the whole
/// point of slot-indexed authority is that this is the only thing it can get
/// wrong about *scope*. *Empty* means it named a slot in its table that nobody
/// filled. The agent gets the same answer for both (a refusal counted as an
/// authority violation); whoever reads the log does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotError {
    /// The slot is beyond the task's capability table.
    OutOfRange { slot: usize, slots: usize },
    /// The slot exists and holds nothing.
    Empty { slot: usize },
}

/// Resolve a slot index against a task's own capability table (ADR-0017 §2).
///
/// This is the whole of what an EL0 agent may name. A raw [`CapId`] lets an
/// agent name any capability in the machine and be stopped by a check; an index
/// into its own array leaves nothing outside that array to name. The
/// unforgeability is in the bound, not in the check — which is why this is a
/// slice lookup and not a comparison against a table of who-holds-what.
///
/// ```
/// use kernel_core::cap::{CapId, SlotError, from_slot};
///
/// let caps = [Some(CapId::new(1, 7)), None];
/// assert_eq!(from_slot(&caps, 0), Ok(CapId::new(1, 7)));
/// assert_eq!(from_slot(&caps, 1), Err(SlotError::Empty { slot: 1 }));
/// assert_eq!(
///     from_slot(&caps, 2),
///     Err(SlotError::OutOfRange { slot: 2, slots: 2 })
/// );
/// ```
#[inline]
pub const fn from_slot(caps: &[Option<CapId>], slot: usize) -> Result<CapId, SlotError> {
    if slot >= caps.len() {
        return Err(SlotError::OutOfRange {
            slot,
            slots: caps.len(),
        });
    }
    match caps[slot] {
        Some(cap) => Ok(cap),
        None => Err(SlotError::Empty { slot }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_slot_is_in_range_and_the_next_one_is_not() {
        // The off-by-one that matters: `slot >= len` and `slot > len` differ
        // only here, and the difference is an agent reading one word past its
        // own capability table.
        let caps = [Some(CapId::new(1, 1)), Some(CapId::new(2, 1))];
        assert_eq!(from_slot(&caps, 1), Ok(CapId::new(2, 1)));
        assert_eq!(
            from_slot(&caps, 2),
            Err(SlotError::OutOfRange { slot: 2, slots: 2 })
        );
    }

    #[test]
    fn an_empty_slot_is_not_an_out_of_range_slot() {
        // Same refusal for the agent, different fact for the reader: one is a
        // grant that never happened, the other is an agent naming a table that
        // is not its own size.
        let caps = [None, Some(CapId::new(9, 3))];
        assert_eq!(from_slot(&caps, 0), Err(SlotError::Empty { slot: 0 }));
        assert!(matches!(
            from_slot(&caps, 7),
            Err(SlotError::OutOfRange { .. })
        ));
    }

    #[test]
    fn an_empty_table_names_nothing_at_all() {
        // A task spawned with no capabilities. Every slot is out of range,
        // including slot 0 — there is no "default" capability to fall back to.
        let caps: [Option<CapId>; 0] = [];
        assert_eq!(
            from_slot(&caps, 0),
            Err(SlotError::OutOfRange { slot: 0, slots: 0 })
        );
    }

    #[test]
    fn pack_unpack() {
        let c = CapId::new(3, 0xAB);
        assert_eq!(c.index(), 3);
        assert_eq!(c.generation(), 0xAB);
        assert_eq!(CapId::from_raw(c.raw()), c);
    }

    #[test]
    fn an_id_survives_the_extremes_of_both_fields() {
        // The two fields are `u16` and the struct is `u32`: a generation of
        // 0xFFFF must not bleed into the index, and vice versa.
        for (index, generation) in [(0u16, 0u16), (0xFFFF, 0), (0, 0xFFFF), (0xFFFF, 0xFFFF)] {
            let c = CapId::new(index, generation);
            assert_eq!(c.index(), index, "index for {index:#x}/{generation:#x}");
            assert_eq!(
                c.generation(),
                generation,
                "gen for {index:#x}/{generation:#x}"
            );
            assert_eq!(CapId::from_raw(c.raw()), c);
        }
    }

    #[test]
    fn ids_differing_only_in_generation_are_distinct() {
        // This is the whole point of the generation field: the same slot, at
        // two different times, must not compare equal. `ipc::lookup_endpoint`
        // relies on it, and no code path exercises that today — see the module
        // docs.
        assert_ne!(CapId::new(7, 1), CapId::new(7, 2));
        assert_eq!(CapId::new(7, 1).index(), CapId::new(7, 2).index());
    }

    #[test]
    fn empty_rights_grant_nothing_and_are_contained_by_everything() {
        let none = CapRights::empty();
        assert!(!none.contains(CapRights::SEND));
        assert!(!none.contains(CapRights::RECV));
        // A required set of "nothing" is satisfied by any holder — the identity
        // `lookup_endpoint` would rely on if it were ever asked for no rights.
        assert!(none.contains(none));
        assert!(CapRights::SEND.contains(none));
    }

    #[test]
    fn send_and_recv_are_distinct_bits() {
        // If these ever collided, every send capability would also be a receive
        // capability and `lookup_endpoint` would stop distinguishing them.
        assert!(!CapRights::SEND.contains(CapRights::RECV));
        assert!(!CapRights::RECV.contains(CapRights::SEND));
        assert!(
            CapRights::SEND
                .union(CapRights::RECV)
                .contains(CapRights::SEND)
        );

        // Each right is a bit that is actually set. Asserting only that the two
        // differ let a mutation turn `RECV` into the empty set and stay green:
        // an empty right is contained by everything, so every check against it
        // would have passed.
        assert!(!CapRights::empty().contains(CapRights::SEND));
        assert!(!CapRights::empty().contains(CapRights::RECV));
        assert!(CapRights::SEND.contains(CapRights::SEND));
        assert!(CapRights::RECV.contains(CapRights::RECV));
        assert!(!CapRights::SEND.contains(CapRights::IRQ));
        assert!(!CapRights::RECV.contains(CapRights::IRQ));
        assert!(!CapRights::IRQ.contains(CapRights::SEND));
        assert!(!CapRights::empty().contains(CapRights::IRQ));
        assert!(CapRights::IRQ.contains(CapRights::IRQ));
    }

    #[test]
    fn union_is_a_union_and_not_a_difference() {
        // Overlapping rights, which the disjoint SEND/RECV pair cannot show:
        // `|` and `^` agree on disjoint bits and disagree here. Under `^`,
        // granting a right twice would silently revoke it.
        assert_eq!(CapRights::SEND.union(CapRights::SEND), CapRights::SEND);
        let both = CapRights::SEND.union(CapRights::RECV);
        assert_eq!(both.union(CapRights::SEND), both);
        assert!(both.union(CapRights::SEND).contains(CapRights::SEND));
    }

    #[test]
    fn rights_contains() {
        let r = CapRights::SEND.union(CapRights::RECV);
        assert!(r.contains(CapRights::SEND));
        assert!(r.contains(CapRights::RECV));
        assert!(!CapRights::SEND.contains(CapRights::RECV));
    }
}
