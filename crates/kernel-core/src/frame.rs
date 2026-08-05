//! Physical frame free-list arithmetic (ADR-0012).
//!
//! Tracks a fixed pool of same-sized frames by **index** only. The kernel maps
//! index → physical address; this module never sees MMIO or page tables.
//!
//! Contract:
//! - [`FramePool::alloc`] hands out an index that was free;
//! - [`FramePool::free`] accepts only indices this pool previously allocated
//!   (double-free and foreign indices are refused, like the heap free-list);
//! - capacity is fixed at construction; no grow/shrink.

/// Maximum frames a single [`FramePool`] can manage (lab M5 sizing headroom).
pub const MAX_FRAMES: usize = 1024;

/// Opaque frame index into a pool (`0 .. capacity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameId(u32);

impl FrameId {
    /// Index as `u32`.
    #[inline]
    pub const fn index(self) -> u32 {
        self.0
    }

    /// Build from a raw index (caller must ensure it is in range for the pool).
    #[inline]
    pub const fn from_index(index: u32) -> Self {
        Self(index)
    }
}

/// Why [`FramePool::free`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameFreeError {
    /// Index ≥ capacity.
    OutOfBounds,
    /// Index is not currently allocated (double-free or never allocated).
    NotAllocated,
}

/// Fixed-capacity pool of frames.
///
/// Free frames sit on an explicit stack; allocated frames are marked in a
/// bitset so free can refuse double-free without scanning the stack.
#[derive(Clone, Debug)]
pub struct FramePool {
    capacity: u32,
    /// Free stack: `free_stack[0..free_len]` are free indices.
    free_stack: [u32; MAX_FRAMES],
    free_len: u32,
    /// Bit `i` set ⇒ frame `i` is currently allocated (handed out).
    allocated: [u64; MAX_FRAMES.div_ceil(64)],
}

impl FramePool {
    /// Empty pool that cannot allocate (capacity 0).
    pub const fn empty() -> Self {
        Self {
            capacity: 0,
            free_stack: [0; MAX_FRAMES],
            free_len: 0,
            allocated: [0; MAX_FRAMES.div_ceil(64)],
        }
    }

    /// Pool of `capacity` frames, all free. `capacity` is clamped to [`MAX_FRAMES`].
    pub fn new(capacity: u32) -> Self {
        let capacity = capacity.min(MAX_FRAMES as u32);
        let mut pool = Self {
            capacity,
            free_stack: [0; MAX_FRAMES],
            free_len: capacity,
            allocated: [0; MAX_FRAMES.div_ceil(64)],
        };
        // Push high indices first so alloc returns low indices first (stable tests).
        for i in 0..capacity {
            pool.free_stack[i as usize] = capacity - 1 - i;
        }
        pool
    }

    /// Configured capacity.
    #[inline]
    pub const fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Frames still free.
    #[inline]
    pub const fn free_count(&self) -> u32 {
        self.free_len
    }

    /// Frames currently handed out.
    #[inline]
    pub const fn used_count(&self) -> u32 {
        self.capacity.saturating_sub(self.free_len)
    }

    /// Allocate one frame, or `None` if exhausted.
    pub fn alloc(&mut self) -> Option<FrameId> {
        if self.free_len == 0 {
            return None;
        }
        self.free_len -= 1;
        let idx = self.free_stack[self.free_len as usize];
        self.set_allocated(idx, true);
        Some(FrameId(idx))
    }

    /// Return a frame previously obtained from [`Self::alloc`].
    pub fn free(&mut self, id: FrameId) -> Result<(), FrameFreeError> {
        let idx = id.0;
        if idx >= self.capacity {
            return Err(FrameFreeError::OutOfBounds);
        }
        if !self.is_allocated(idx) {
            return Err(FrameFreeError::NotAllocated);
        }
        self.set_allocated(idx, false);
        debug_assert!(self.free_len < self.capacity);
        self.free_stack[self.free_len as usize] = idx;
        self.free_len += 1;
        Ok(())
    }

    fn word_bit(idx: u32) -> (usize, u64) {
        let i = idx as usize;
        (i / 64, 1u64 << (i % 64))
    }

    fn is_allocated(&self, idx: u32) -> bool {
        let (w, bit) = Self::word_bit(idx);
        self.allocated[w] & bit != 0
    }

    fn set_allocated(&mut self, idx: u32, on: bool) {
        let (w, bit) = Self::word_bit(idx);
        if on {
            self.allocated[w] |= bit;
        } else {
            self.allocated[w] &= !bit;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_exhausts_then_none() {
        let mut p = FramePool::new(3);
        assert_eq!(p.free_count(), 3);
        assert!(p.alloc().is_some());
        assert!(p.alloc().is_some());
        assert!(p.alloc().is_some());
        assert_eq!(p.free_count(), 0);
        assert_eq!(p.used_count(), 3);
        assert!(p.alloc().is_none());
    }

    #[test]
    fn free_returns_to_pool() {
        let mut p = FramePool::new(2);
        let a = p.alloc().unwrap();
        let b = p.alloc().unwrap();
        assert!(p.alloc().is_none());
        p.free(a).unwrap();
        assert_eq!(p.free_count(), 1);
        let c = p.alloc().unwrap();
        assert_eq!(c, a);
        p.free(b).unwrap();
        p.free(c).unwrap();
        assert_eq!(p.free_count(), 2);
    }

    #[test]
    fn double_free_refused() {
        let mut p = FramePool::new(1);
        let a = p.alloc().unwrap();
        p.free(a).unwrap();
        assert_eq!(p.free(a), Err(FrameFreeError::NotAllocated));
        assert_eq!(p.free_count(), 1);
    }

    #[test]
    fn foreign_index_refused() {
        let mut p = FramePool::new(2);
        assert_eq!(
            p.free(FrameId::from_index(99)),
            Err(FrameFreeError::OutOfBounds)
        );
        assert_eq!(
            p.free(FrameId::from_index(1)),
            Err(FrameFreeError::NotAllocated)
        );
    }

    #[test]
    fn capacity_clamped_to_max() {
        let p = FramePool::new((MAX_FRAMES as u32) + 100);
        assert_eq!(p.capacity(), MAX_FRAMES as u32);
        assert_eq!(p.free_count(), MAX_FRAMES as u32);
    }

    #[test]
    fn empty_pool() {
        let mut p = FramePool::empty();
        assert_eq!(p.capacity(), 0);
        assert!(p.alloc().is_none());
    }

    #[test]
    fn alloc_prefers_low_indices_first() {
        let mut p = FramePool::new(4);
        let a = p.alloc().unwrap().index();
        let b = p.alloc().unwrap().index();
        assert_eq!(a, 0);
        assert_eq!(b, 1);
    }
}
