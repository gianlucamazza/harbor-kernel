//! IRQ notification capabilities (ADR-0030) — pure, host-tested.
//!
//! A notification names an IRQ **cookie** (ADR-0008), not a GIC id. `CapId`
//! indices for this table sit in the high half so they never collide with IPC
//! endpoint indices (`0..MAX_ENDPOINTS`).

use crate::cap::CapId;

/// Concurrent notification objects.
pub const MAX_IRQ_CAPS: usize = 8;

/// High bit set: IRQ-cap indices are `INDEX_BASE | local` (local < MAX_IRQ_CAPS).
pub const INDEX_BASE: u16 = 0x8000;

/// Why [`Table::mint`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MintError {
    Full,
}

/// Why [`Table::lookup`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupError {
    BadCap,
}

#[derive(Clone, Copy)]
struct Entry {
    live: bool,
    generation: u16,
    cookie: u32,
}

impl Entry {
    const EMPTY: Self = Self {
        live: false,
        generation: 0,
        cookie: 0,
    };
}

/// Pure table: mint cookie → CapId, lookup CapId → cookie.
#[derive(Clone)]
pub struct Table {
    entries: [Entry; MAX_IRQ_CAPS],
}

impl Table {
    pub const fn new() -> Self {
        Self {
            entries: [Entry::EMPTY; MAX_IRQ_CAPS],
        }
    }

    /// Mint a notification for `cookie`. Returns a CapId the holder may wait on.
    pub fn mint(&mut self, cookie: u32) -> Result<CapId, MintError> {
        for i in 0..MAX_IRQ_CAPS {
            if !self.entries[i].live {
                let mut generation = self.entries[i].generation.wrapping_add(1);
                // generation 0 is never live after first mint (same as ipc style).
                if generation == 0 {
                    generation = 1;
                }
                self.entries[i] = Entry {
                    live: true,
                    generation,
                    cookie,
                };
                return Ok(CapId::new(INDEX_BASE | (i as u16), generation));
            }
        }
        Err(MintError::Full)
    }

    /// Resolve a CapId to its cookie if the entry is live and generation matches.
    pub fn lookup(&self, cap: CapId) -> Result<u32, LookupError> {
        // ADR-0059: class decode, not a band mask — the both-bands quadrant
        // used to fall through to the local-bound check and now refuses as
        // what it is.
        let local = match cap.classify() {
            crate::cap::CapClass::Irq(local) => local as usize,
            _ => return Err(LookupError::BadCap),
        };
        if local >= MAX_IRQ_CAPS {
            return Err(LookupError::BadCap);
        }
        let e = &self.entries[local];
        if !e.live || e.generation != cap.generation() {
            return Err(LookupError::BadCap);
        }
        Ok(e.cookie)
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cap::CapId;

    #[test]
    fn low_shaped_id_never_hits_a_live_entry() {
        // Same local index and generation as a live entry, band bit stripped:
        // the decode must refuse before the entry can match (the empty-table
        // variant could not tell the two apart — cargo-mutants showed it).
        let mut t = Table::new();
        let cap = t.mint(42).unwrap();
        let low = CapId::new(cap.index() & !INDEX_BASE, cap.generation());
        assert_eq!(t.lookup(low), Err(LookupError::BadCap));
    }

    #[test]
    fn mint_lookup_round_trip() {
        let mut t = Table::new();
        let cap = t.mint(1).unwrap();
        assert_eq!(t.lookup(cap), Ok(1));
        assert!(cap.index() & INDEX_BASE != 0);
    }

    #[test]
    fn ipc_style_index_is_refused() {
        let t = Table::new();
        // Endpoint-shaped CapId must not resolve as an IRQ notification.
        assert_eq!(t.lookup(CapId::new(0, 1)), Err(LookupError::BadCap));
    }

    #[test]
    fn stale_generation_is_refused() {
        let mut t = Table::new();
        let cap = t.mint(2).unwrap();
        let stale = CapId::new(cap.index(), cap.generation().wrapping_add(1));
        assert_eq!(t.lookup(stale), Err(LookupError::BadCap));
        assert_eq!(t.lookup(cap), Ok(2));
    }

    #[test]
    fn full_table_refuses() {
        let mut t = Table::new();
        for i in 0..MAX_IRQ_CAPS {
            t.mint(i as u32).unwrap();
        }
        assert_eq!(t.mint(99), Err(MintError::Full));
    }

    #[test]
    fn two_cookies_two_caps() {
        let mut t = Table::new();
        let a = t.mint(1).unwrap();
        let b = t.mint(2).unwrap();
        assert_ne!(a, b);
        assert_eq!(t.lookup(a), Ok(1));
        assert_eq!(t.lookup(b), Ok(2));
    }
}
