//! Single-producer / single-consumer byte ring.
//!
//! Capacity is `N - 1` usable bytes (`N` must be a power of two ≥ 2). One slot
//! is left empty so full and empty are distinguishable without a third flag.
//!
//! Intended for IRQ producer → main/consumer on a single core: the producer
//! only mutates `head`, the consumer only mutates `tail`.

/// Fixed-capacity SPSC byte queue.
pub struct ByteRing<const N: usize> {
    buf: [u8; N],
    head: usize,
    tail: usize,
}

impl<const N: usize> ByteRing<N> {
    /// Empty ring. `N` must be a power of two and at least 2.
    pub const fn new() -> Self {
        // const panic if mis-sized (edition 2021).
        assert!(N >= 2 && N.is_power_of_two());
        Self {
            buf: [0; N],
            head: 0,
            tail: 0,
        }
    }

    #[inline]
    const fn mask() -> usize {
        N - 1
    }

    /// Push one byte. Returns `false` if the ring is full (byte not stored).
    #[inline]
    pub fn push(&mut self, byte: u8) -> bool {
        let next = (self.head + 1) & Self::mask();
        if next == self.tail {
            return false;
        }
        self.buf[self.head] = byte;
        // Ensure the byte is visible before the head update (IRQ producer /
        // main consumer on one core; fence blocks compiler reordering only).
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);
        self.head = next;
        true
    }

    /// Pop one byte, or `None` if empty.
    #[inline]
    pub fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            return None;
        }
        let byte = self.buf[self.tail];
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Acquire);
        self.tail = (self.tail + 1) & Self::mask();
        Some(byte)
    }

    /// `true` when there are no bytes to pop.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Bytes currently stored (not capacity).
    #[inline]
    pub fn len(&self) -> usize {
        (self.head.wrapping_sub(self.tail)) & Self::mask()
    }
}

impl<const N: usize> Default for ByteRing<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pop_is_none() {
        let mut r = ByteRing::<8>::new();
        assert!(r.is_empty());
        assert_eq!(r.pop(), None);
    }

    #[test]
    fn push_pop_fifo() {
        let mut r = ByteRing::<8>::new();
        assert!(r.push(1));
        assert!(r.push(2));
        assert_eq!(r.len(), 2);
        assert_eq!(r.pop(), Some(1));
        assert_eq!(r.pop(), Some(2));
        assert!(r.is_empty());
    }

    #[test]
    fn full_rejects_without_overwrite() {
        let mut r = ByteRing::<4>::new();
        assert!(r.push(10));
        assert!(r.push(20));
        assert!(r.push(30));
        // Capacity N-1 = 3.
        assert!(!r.push(40));
        assert_eq!(r.pop(), Some(10));
        assert!(r.push(40));
        assert_eq!(r.pop(), Some(20));
        assert_eq!(r.pop(), Some(30));
        assert_eq!(r.pop(), Some(40));
    }

    #[test]
    fn wraps_indices() {
        let mut r = ByteRing::<4>::new();
        for i in 0..20u8 {
            assert!(r.push(i));
            assert_eq!(r.pop(), Some(i));
        }
    }
}
