//! Bump allocator bookkeeping.
//!
//! Address arithmetic only — the kernel wraps this with the actual pointer and
//! the `static mut`/lock discipline. Keeping the arithmetic here means the
//! overflow and alignment edge cases are testable without a board.

/// A half-open region `[cur, end)` handed out from the bottom up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bump {
    cur: usize,
    end: usize,
}

impl Bump {
    /// An exhausted allocator, usable as a `const` initialiser before boot.
    pub const fn empty() -> Self {
        Self { cur: 0, end: 0 }
    }

    /// Build an allocator over `[start, end)`. `None` if the region is empty.
    pub const fn new(start: usize, end: usize) -> Option<Self> {
        if end <= start {
            return None;
        }
        Some(Self { cur: start, end })
    }

    /// Bytes still available (ignoring alignment padding).
    pub const fn remaining(&self) -> usize {
        self.end.saturating_sub(self.cur)
    }

    /// Next address that would be handed out for `align`.
    pub const fn cursor(&self) -> usize {
        self.cur
    }

    /// Carve out `size` bytes aligned to `align`.
    ///
    /// Returns the base address, or `None` if `align` is not a power of two or
    /// the region cannot satisfy the request.
    pub fn alloc(&mut self, size: usize, align: usize) -> Option<usize> {
        if !align.is_power_of_two() {
            return None;
        }
        // Checked: near the top of the address space the round-up wraps, and a
        // wrapped base compares below `end`.
        let aligned = self.cur.checked_add(align - 1)? & !(align - 1);
        let next = aligned.checked_add(size)?;
        if next > self.end {
            return None;
        }
        self.cur = next;
        Some(aligned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_are_sequential_and_aligned() {
        let mut b = Bump::new(0x1000, 0x2000).unwrap();
        assert_eq!(b.alloc(8, 16), Some(0x1000));
        assert_eq!(b.alloc(8, 16), Some(0x1010));
        assert_eq!(b.alloc(1, 64), Some(0x1040));
    }

    #[test]
    fn empty_region_is_rejected() {
        assert_eq!(Bump::new(0x1000, 0x1000), None);
        assert_eq!(Bump::new(0x2000, 0x1000), None);
    }

    #[test]
    fn exhaustion_returns_none_and_leaves_the_cursor_intact() {
        let mut b = Bump::new(0x1000, 0x1100).unwrap();
        assert_eq!(b.alloc(0x100, 1), Some(0x1000));
        let before = b.cursor();
        assert_eq!(b.alloc(1, 1), None);
        assert_eq!(b.cursor(), before, "a failed alloc must not consume space");
    }

    /// The alignment round-up is `cur + (align - 1)`. Near the top of the
    /// address space that addition wraps, and a wrapped base compares below
    /// `end`, so the allocator would hand out a pointer outside its region.
    #[test]
    fn alignment_round_up_must_not_wrap() {
        let mut b = Bump::new(usize::MAX - 8, usize::MAX).unwrap();
        assert_eq!(b.alloc(16, 4096), None);
    }

    /// Same wrap, reached through `size` instead of alignment.
    #[test]
    fn size_addition_must_not_wrap() {
        let mut b = Bump::new(0x1000, usize::MAX).unwrap();
        assert_eq!(b.alloc(usize::MAX, 16), None);
    }

    /// A bad alignment is a caller error, but a kernel allocator answering it
    /// with a panic turns a recoverable request into a dead board.
    #[test]
    fn non_power_of_two_alignment_is_rejected_not_a_panic() {
        let mut b = Bump::new(0x1000, 0x2000).unwrap();
        assert_eq!(b.alloc(8, 24), None);
        assert_eq!(b.alloc(8, 0), None);
    }

    #[test]
    fn remaining_tracks_consumption() {
        let mut b = Bump::new(0x1000, 0x1100).unwrap();
        assert_eq!(b.remaining(), 0x100);
        b.alloc(0x40, 1).unwrap();
        assert_eq!(b.remaining(), 0xC0);
    }
}
