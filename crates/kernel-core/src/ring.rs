//! Single-producer / single-consumer byte ring.
//!
//! Capacity is `N - 1` usable bytes (`N` must be a power of two ≥ 2). One slot
//! is left empty so full and empty are distinguishable without a third flag.
//!
//! # Why `&self` and not `&mut self`
//!
//! The producer is an IRQ handler and the consumer is the main loop. With a
//! `&mut self` API both sides must reconstruct a `&mut` from the same `static`,
//! and an interrupt taken mid-`pop` makes two `&mut` to one object live at
//! once — aliasing UB that no fence can repair. Taking `&self` and putting the
//! indices in atomics means that code cannot be written in the first place.
//!
//! The ordering is Lamport's SPSC queue: the producer owns `head`, the
//! consumer owns `tail`, and each publishes its index with `Release` after the
//! data access, reading the other's with `Acquire` before it. That is correct
//! across cores too, so this survives the move to SMP unchanged.

#![allow(unsafe_code)] // see the module docs: `UnsafeCell` buffer + `Sync` assertion

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

/// Fixed-capacity SPSC byte queue.
pub struct ByteRing<const N: usize> {
    buf: UnsafeCell<[u8; N]>,
    /// Next slot to write. Mutated only by the producer.
    head: AtomicUsize,
    /// Next slot to read. Mutated only by the consumer.
    tail: AtomicUsize,
}

// SAFETY: the only mutable access to `buf[i]` is by the producer before it
// publishes `head`, and by the consumer before it publishes `tail`; the
// Acquire/Release pair on those indices means the two never touch the same
// slot concurrently.
unsafe impl<const N: usize> Sync for ByteRing<N> {}

impl<const N: usize> ByteRing<N> {
    /// Empty ring. `N` must be a power of two and at least 2.
    pub const fn new() -> Self {
        assert!(N >= 2 && N.is_power_of_two());
        Self {
            buf: UnsafeCell::new([0; N]),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    #[inline]
    const fn mask() -> usize {
        N - 1
    }

    /// Push one byte. Returns `false` if the ring is full (byte not stored).
    ///
    /// Only one producer may call this.
    #[inline]
    pub fn push(&self, byte: u8) -> bool {
        // Relaxed: this thread is the only writer of `head`.
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) & Self::mask();

        // Acquire: pairs with the consumer's Release store, so the slot it
        // freed is visible before we reuse it.
        if next == self.tail.load(Ordering::Acquire) {
            return false;
        }

        // SAFETY: `head` is ours until published below, and the consumer never
        // reads this slot until it observes the new `head`.
        unsafe {
            (*self.buf.get())[head] = byte;
        }

        // Release: publishes the byte written above. Publishing before the
        // write lets the consumer read an empty slot — the two-thread test
        // below catches exactly that.
        self.head.store(next, Ordering::Release);
        true
    }

    /// Pop one byte, or `None` if empty.
    ///
    /// Only one consumer may call this.
    #[inline]
    pub fn pop(&self) -> Option<u8> {
        // Relaxed: this thread is the only writer of `tail`.
        let tail = self.tail.load(Ordering::Relaxed);

        // Acquire: pairs with the producer's Release store, so the byte it
        // wrote is visible before we read the slot.
        if tail == self.head.load(Ordering::Acquire) {
            return None;
        }

        // SAFETY: the producer will not write this slot until it observes the
        // new `tail` published below.
        let byte = unsafe { (*self.buf.get())[tail] };

        // Release: frees the slot for the producer.
        self.tail
            .store((tail + 1) & Self::mask(), Ordering::Release);
        Some(byte)
    }

    /// `true` when there are no bytes to pop.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tail.load(Ordering::Relaxed) == self.head.load(Ordering::Acquire)
    }

    /// Bytes currently stored (not capacity). A snapshot: with a concurrent
    /// producer the real count can only be larger by the time it is read.
    #[inline]
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);
        head.wrapping_sub(tail) & Self::mask()
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
        let r = ByteRing::<8>::new();
        assert!(r.is_empty());
        assert_eq!(r.pop(), None);
    }

    #[test]
    fn push_pop_fifo() {
        let r = ByteRing::<8>::new();
        assert!(r.push(1));
        assert!(r.push(2));
        assert_eq!(r.len(), 2);
        assert_eq!(r.pop(), Some(1));
        assert_eq!(r.pop(), Some(2));
        assert!(r.is_empty());
    }

    #[test]
    fn full_rejects_without_overwrite() {
        let r = ByteRing::<4>::new();
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
        let r = ByteRing::<4>::new();
        for i in 0..20u8 {
            assert!(r.push(i));
            assert_eq!(r.pop(), Some(i));
        }
    }

    /// Real concurrency, which a `&mut self` API cannot express at all.
    ///
    /// A producer thread and a consumer thread run against one shared ring.
    /// The consumer checks that every byte arrives exactly once and in order:
    /// a lost Release on `head` lets the consumer read a slot before the byte
    /// lands in it, and the sequence breaks.
    #[test]
    fn spsc_across_two_threads_preserves_the_sequence() {
        use std::sync::Arc;
        use std::thread;

        const COUNT: usize = 200_000;
        // Bounded so a broken ring fails the run instead of hanging it: if one
        // side dies the other would otherwise spin on a permanently full or
        // permanently empty queue forever.
        const SPIN_LIMIT: usize = 10_000_000;

        let ring = Arc::new(ByteRing::<64>::new());

        let producer = {
            let ring = Arc::clone(&ring);
            thread::spawn(move || {
                for i in 0..COUNT {
                    // Spin until there is room: `push` must never overwrite.
                    let mut spins = 0;
                    while !ring.push((i % 251) as u8) {
                        spins += 1;
                        assert!(spins < SPIN_LIMIT, "producer stuck on a full ring at {i}");
                        std::hint::spin_loop();
                    }
                }
            })
        };

        let consumer = thread::spawn(move || {
            for i in 0..COUNT {
                let mut spins = 0;
                loop {
                    if let Some(byte) = ring.pop() {
                        assert_eq!(byte, (i % 251) as u8, "out of sequence at {i}");
                        break;
                    }
                    spins += 1;
                    assert!(spins < SPIN_LIMIT, "consumer stuck on an empty ring at {i}");
                    std::hint::spin_loop();
                }
            }
        });

        // Join both before unwrapping, so one thread's panic cannot leave the
        // other spinning against a queue nobody drains.
        let producer_result = producer.join();
        let consumer_result = consumer.join();
        producer_result.expect("producer panicked");
        consumer_result.expect("consumer panicked");
    }
}
