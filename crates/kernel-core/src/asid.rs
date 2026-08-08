//! ASID pool and TTBR0 packing (ADR-0047 / ADR-0050 / K7) — pure, host-tested.
//!
//! AArch64 stage-1 takes the ASID from `TTBR0_EL1[63:48]` when `TCR_EL1.A1=0`
//! (the regime this kernel uses). ASID 0 is reserved for the kernel root so
//! identity/global leaves stay untagged. User address spaces take a free ASID
//! from a fixed 8-bit pool.

/// Bits of ASID width programmed into `TCR_EL1` (AS=0 → 8-bit).
pub const ASID_BITS: u32 = 8;

/// Number of distinct ASID values (`0..ASID_COUNT`).
pub const ASID_COUNT: usize = 1 << ASID_BITS;

/// Bit position of the ASID field in `TTBR0_EL1` / `TTBR1_EL1`.
pub const ASID_SHIFT: u32 = 48;

/// Mask for the ASID field once shifted down.
pub const ASID_MASK: u16 = (ASID_COUNT as u16) - 1;

/// Reserved for the kernel identity root (global / unmarked regime).
pub const KERNEL_ASID: u16 = 0;

/// Physical root bits that may sit under the ASID field (BADDR + CnP).
const ROOT_MASK: u64 = (1u64 << ASID_SHIFT) - 1;

/// Pack a page-table root physical address with an ASID for `TTBR0_EL1`.
#[inline]
pub const fn pack_ttbr0(root_phys: u64, asid: u16) -> u64 {
    (root_phys & ROOT_MASK) | (((asid as u64) & (ASID_MASK as u64)) << ASID_SHIFT)
}

/// Recover the physical root from a packed TTBR0 value.
#[inline]
pub const fn unpack_root(ttbr: u64) -> u64 {
    ttbr & ROOT_MASK
}

/// Recover the ASID from a packed TTBR0 value.
#[inline]
pub const fn unpack_asid(ttbr: u64) -> u16 {
    ((ttbr >> ASID_SHIFT) as u16) & ASID_MASK
}

/// Fixed-size free-list of non-kernel ASIDs.
///
/// ASID 0 is never allocated. Exhaustion returns `None` rather than wrapping
/// or reusing a live tag (reuse is the caller's responsibility after free +
/// TLBI by ASID).
#[derive(Clone, Debug)]
pub struct AsidPool {
    /// Bit `i` set ⇒ ASID `i` is free. Bit 0 is always clear (reserved).
    free: [u64; ASID_COUNT / 64],
    /// Next index to scan (round-robin hint).
    next: u16,
    /// How many non-kernel ASIDs are currently free.
    free_count: u16,
}

impl AsidPool {
    /// Fresh pool: every ASID except [`KERNEL_ASID`] is free.
    pub const fn new() -> Self {
        // 256 bits, four u64 words, all ones then clear bit 0.
        let mut free = [u64::MAX; ASID_COUNT / 64];
        free[0] &= !1; // reserve ASID 0
        Self {
            free,
            next: 1,
            free_count: (ASID_COUNT as u16) - 1,
        }
    }

    /// Allocate one free ASID, or `None` if the pool is empty.
    pub fn alloc(&mut self) -> Option<u16> {
        if self.free_count == 0 {
            return None;
        }
        let start = self.next as usize % ASID_COUNT;
        let mut i = start;
        loop {
            if i != KERNEL_ASID as usize && self.is_free(i as u16) {
                self.mark_used(i as u16);
                self.free_count -= 1;
                self.next = ((i + 1) % ASID_COUNT) as u16;
                return Some(i as u16);
            }
            i = (i + 1) % ASID_COUNT;
            if i == start {
                // free_count said otherwise — defensive.
                return None;
            }
        }
    }

    /// Return `asid` to the pool. No-op for [`KERNEL_ASID`] or already-free.
    ///
    /// Returns `true` if the ASID was live and is now free (caller must
    /// invalidate TLB entries tagged with this ASID before reuse).
    pub fn free(&mut self, asid: u16) -> bool {
        if asid == KERNEL_ASID || asid as usize >= ASID_COUNT {
            return false;
        }
        if self.is_free(asid) {
            return false;
        }
        self.mark_free(asid);
        self.free_count = self.free_count.saturating_add(1);
        true
    }

    /// ASIDs still available for user address spaces.
    #[inline]
    pub const fn free_count(&self) -> u16 {
        self.free_count
    }

    /// True if `asid` is currently free (or out of range / kernel reserved).
    #[inline]
    pub fn is_free(&self, asid: u16) -> bool {
        if asid as usize >= ASID_COUNT {
            return true;
        }
        let word = asid as usize / 64;
        let bit = asid as usize % 64;
        (self.free[word] >> bit) & 1 == 1
    }

    #[inline]
    fn mark_used(&mut self, asid: u16) {
        let word = asid as usize / 64;
        let bit = asid as usize % 64;
        self.free[word] &= !(1u64 << bit);
    }

    #[inline]
    fn mark_free(&mut self, asid: u16) {
        let word = asid as usize / 64;
        let bit = asid as usize % 64;
        self.free[word] |= 1u64 << bit;
    }
}

impl Default for AsidPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trip() {
        let root = 0x4_0000_u64;
        let asid = 7u16;
        let ttbr = pack_ttbr0(root, asid);
        assert_eq!(unpack_root(ttbr), root);
        assert_eq!(unpack_asid(ttbr), asid);
        assert_eq!(pack_ttbr0(root, KERNEL_ASID), root);
    }

    #[test]
    fn kernel_asid_never_allocated() {
        let mut p = AsidPool::new();
        assert_eq!(p.free_count(), 255);
        for _ in 0..255 {
            let a = p.alloc().expect("pool should yield");
            assert_ne!(a, KERNEL_ASID);
        }
        assert!(p.alloc().is_none());
        assert_eq!(p.free_count(), 0);
    }

    #[test]
    fn free_then_reuse() {
        let mut p = AsidPool::new();
        let a = p.alloc().unwrap();
        let b = p.alloc().unwrap();
        assert_ne!(a, b);
        assert!(p.free(a));
        assert!(!p.free(a), "double free is a no-op");
        assert!(!p.free(KERNEL_ASID));
        // Round-robin may hand out a higher tag first; drain until `a` returns.
        let mut saw_a = false;
        for _ in 0..ASID_COUNT {
            match p.alloc() {
                Some(x) if x == a => {
                    saw_a = true;
                    break;
                }
                Some(_) => {}
                None => break,
            }
        }
        assert!(saw_a, "freed ASID must become allocatable again");
        let _ = b;
    }

    #[test]
    fn asid_field_masks_to_8_bits() {
        let ttbr = pack_ttbr0(0x1000, 0x1FF);
        assert_eq!(unpack_asid(ttbr), 0xFF);
    }
}
