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
//! struct: EL0 has no syscall that takes a capability at all, and inside the
//! kernel `ipc::lookup_endpoint` checks the id against a live table entry with
//! a matching generation, while `sched::current_holds` checks that the calling
//! task was given it. The module used to call this "the unforgeable id", which
//! reads as though the type carried the guarantee.
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

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn rights_contains() {
        let r = CapRights::SEND.union(CapRights::RECV);
        assert!(r.contains(CapRights::SEND));
        assert!(r.contains(CapRights::RECV));
        assert!(!CapRights::SEND.contains(CapRights::RECV));
    }
}
