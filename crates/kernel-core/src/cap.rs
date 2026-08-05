//! Capability id encoding (pure arithmetic for M4).
//!
//! A [`CapId`] packs a table index and a generation so a recycled slot cannot
//! be confused with a stale handle. Rights bits travel with the endpoint table
//! entry in the kernel crate; this module only frames the unforgeable id.

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
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    #[inline]
    pub const fn bits(self) -> u8 {
        self.0
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
    fn rights_contains() {
        let r = CapRights::SEND.union(CapRights::RECV);
        assert!(r.contains(CapRights::SEND));
        assert!(r.contains(CapRights::RECV));
        assert!(!CapRights::SEND.contains(CapRights::RECV));
    }
}
