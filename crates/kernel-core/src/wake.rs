//! Single-producer / single-consumer wake queue (ADR-0008).
//!
//! The producer is an IRQ handler; the consumer is the voluntary path (idle /
//! scheduler). Same Lamport SPSC rules as [`crate::ring::ByteRing`]: one empty
//! slot, `Release` after write / `Acquire` before read, no `&mut` shared across
//! IRQ and main.
//!
//! Entries are opaque `u32` tokens (typically a [`crate::runqueue::TaskId`]).
//! Capacity is fixed; a full queue drops the wake and counts it rather than
//! spinning in IRQ context.

#![allow(unsafe_code)] // `UnsafeCell` buffer + `Sync` for IRQ/main SPSC

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// Fixed-capacity SPSC queue of wake tokens.
pub struct WakeQueue<const N: usize> {
    slots: UnsafeCell<[u32; N]>,
    /// Next write index. Producer only.
    head: AtomicUsize,
    /// Next read index. Consumer only.
    tail: AtomicUsize,
    /// Wakes discarded because the queue was full.
    drops: AtomicU32,
}

// SAFETY: producer alone mutates slots it owns until `head` is published;
// consumer alone reads after observing `head` and frees with `tail`.
unsafe impl<const N: usize> Sync for WakeQueue<N> {}

impl<const N: usize> WakeQueue<N> {
    /// Empty queue. `N` must be a power of two ≥ 2.
    pub const fn new() -> Self {
        assert!(N >= 2 && N.is_power_of_two());
        Self {
            slots: UnsafeCell::new([0; N]),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            drops: AtomicU32::new(0),
        }
    }

    #[inline]
    const fn mask() -> usize {
        N - 1
    }

    /// Push one wake token. Returns `false` if full (token dropped, counter++).
    ///
    /// Only one producer (IRQ path) may call this.
    #[inline]
    pub fn push(&self, token: u32) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) & Self::mask();
        if next == self.tail.load(Ordering::Acquire) {
            self.drops.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        // SAFETY: slot exclusive to producer until head is published.
        unsafe {
            (*self.slots.get())[head] = token;
        }
        self.head.store(next, Ordering::Release);
        true
    }

    /// Pop one token, or `None` if empty.
    ///
    /// Only one consumer (voluntary path) may call this.
    #[inline]
    pub fn pop(&self) -> Option<u32> {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: producer will not reuse this slot until tail advances.
        let token = unsafe { (*self.slots.get())[tail] };
        self.tail.store((tail + 1) & Self::mask(), Ordering::Release);
        Some(token)
    }

    /// How many pushes failed because the queue was full.
    #[inline]
    pub fn drops(&self) -> u32 {
        self.drops.load(Ordering::Relaxed)
    }

    /// True when empty (consumer view).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tail.load(Ordering::Relaxed) == self.head.load(Ordering::Acquire)
    }
}

impl<const N: usize> Default for WakeQueue<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_round_trip() {
        let q = WakeQueue::<8>::new();
        assert!(q.push(7));
        assert!(q.push(9));
        assert_eq!(q.pop(), Some(7));
        assert_eq!(q.pop(), Some(9));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn full_drops_and_counts() {
        // Usable capacity = N - 1.
        let q = WakeQueue::<4>::new();
        assert!(q.push(1));
        assert!(q.push(2));
        assert!(q.push(3));
        assert!(!q.push(4));
        assert_eq!(q.drops(), 1);
        assert_eq!(q.pop(), Some(1));
        assert!(q.push(5));
        assert_eq!(q.drops(), 1);
    }

    #[test]
    fn empty_after_drain() {
        let q = WakeQueue::<4>::new();
        assert!(q.is_empty());
        assert!(q.push(1));
        assert!(!q.is_empty());
        let _ = q.pop();
        assert!(q.is_empty());
    }
}
