//! First-fit free-list allocator with splitting and coalescing.
//!
//! The bump allocator in [`crate::bump`] cannot free, which is fine for
//! early boot and useless for tasks, mailboxes or anything with a lifetime.
//! This one is the general allocator behind `GlobalAlloc`.
//!
//! # Why it works on a byte slice
//!
//! Bookkeeping lives inside the memory being managed, as it must for a kernel
//! heap — but it is addressed by *offset* into a `&mut [u8]` rather than by
//! raw pointer. That keeps every index check, split, merge and alignment
//! calculation in safe code the host test suite can drive against a `Vec<u8>`,
//! and leaves the kernel with nothing to get wrong but the base address.
//!
//! Layout of the arena:
//!
//! ```text
//! [ header | payload ......... ][ header | payload ... ] ...
//!   ^ block start                ^ next block
//! ```
//!
//! A header is [`HEADER`] bytes: block size, then (free blocks only) the offset
//! of the next free block. Allocated blocks store the block start so `dealloc`
//! can find it even when alignment pushed the payload forward.

/// Bytes of bookkeeping at the start of every block.
pub const HEADER: usize = 16;

/// Allocation granularity. Every block start and size is a multiple of this,
/// which keeps headers naturally aligned and bounds fragmentation.
pub const GRAIN: usize = 16;

/// Smallest block worth leaving behind when splitting.
const MIN_BLOCK: usize = HEADER + GRAIN;

/// Sentinel for "no next block" — no real offset can reach it.
const NIL: u64 = u64::MAX;

/// A free-list allocator over a caller-owned byte arena.
///
/// The arena must be the same slice on every call; the allocator stores only
/// offsets into it.
#[derive(Clone, Copy, Debug)]
pub struct FreeList {
    /// Offset of the first free block, or `None` when full.
    head: Option<usize>,
    /// Total arena length, to catch a caller passing a different slice.
    len: usize,
}

fn read_u64(arena: &[u8], at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&arena[at..at + 8]);
    u64::from_ne_bytes(bytes)
}

fn write_u64(arena: &mut [u8], at: usize, value: u64) {
    arena[at..at + 8].copy_from_slice(&value.to_ne_bytes());
}

fn block_size(arena: &[u8], block: usize) -> usize {
    read_u64(arena, block) as usize
}

fn set_block_size(arena: &mut [u8], block: usize, size: usize) {
    write_u64(arena, block, size as u64);
}

fn next_free(arena: &[u8], block: usize) -> Option<usize> {
    match read_u64(arena, block + 8) {
        NIL => None,
        offset => Some(offset as usize),
    }
}

fn set_next_free(arena: &mut [u8], block: usize, next: Option<usize>) {
    write_u64(arena, block + 8, next.map_or(NIL, |n| n as u64));
}

const fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

impl FreeList {
    /// Take ownership of `arena` as one free block.
    ///
    /// `None` if the arena is too small to hold a single block.
    pub fn new(arena: &mut [u8]) -> Option<Self> {
        let len = arena.len() & !(GRAIN - 1);
        if len < MIN_BLOCK {
            return None;
        }
        set_block_size(arena, 0, len);
        set_next_free(arena, 0, None);
        Some(Self {
            head: Some(0),
            len: arena.len(),
        })
    }

    /// Bytes still free, including per-block headers.
    pub fn free_bytes(&self, arena: &[u8]) -> usize {
        let mut total = 0;
        let mut cursor = self.head;
        while let Some(block) = cursor {
            total += block_size(arena, block);
            cursor = next_free(arena, block);
        }
        total
    }

    /// Number of free blocks — a fragmentation signal.
    pub fn free_blocks(&self, arena: &[u8]) -> usize {
        let mut count = 0;
        let mut cursor = self.head;
        while let Some(block) = cursor {
            count += 1;
            cursor = next_free(arena, block);
        }
        count
    }

    /// Allocate `size` bytes aligned to `align`, returning a payload offset.
    ///
    /// `None` if `align` is not a power of two, the request is empty, or no
    /// block can host it.
    pub fn alloc(&mut self, arena: &mut [u8], size: usize, align: usize) -> Option<usize> {
        if size == 0 || !align.is_power_of_two() || arena.len() != self.len {
            return None;
        }
        let size = align_up(size, GRAIN);

        // First fit, tracking the predecessor so the block can be unlinked.
        let mut prev: Option<usize> = None;
        let mut cursor = self.head;

        while let Some(block) = cursor {
            let block_end = block + block_size(arena, block);
            // The payload starts after the header, pushed up to `align`.
            let payload = align_up(block + HEADER, align);

            if payload.checked_add(size)? <= block_end {
                let next = next_free(arena, block);

                // Anything before the payload beyond the header is padding we
                // cannot hand back; it stays part of this block.
                let tail = payload + size;
                if block_end - tail >= MIN_BLOCK {
                    // Split: the remainder becomes a free block of its own.
                    set_block_size(arena, tail, block_end - tail);
                    set_next_free(arena, tail, next);
                    self.relink(arena, prev, Some(tail));
                    set_block_size(arena, block, tail - block);
                } else {
                    // Too small to split: the whole block goes to the caller.
                    self.relink(arena, prev, next);
                }

                // Record where the block starts so `dealloc` can find it after
                // alignment moved the payload away from `block + HEADER`.
                write_u64(arena, payload - 8, block as u64);
                return Some(payload);
            }

            prev = Some(block);
            cursor = next_free(arena, block);
        }

        None
    }

    /// Return a payload offset previously produced by [`alloc`](Self::alloc).
    ///
    /// Blocks are kept sorted by address and merged with their neighbours, so
    /// a heap that is fully freed goes back to a single block.
    pub fn dealloc(&mut self, arena: &mut [u8], payload: usize) {
        if payload < HEADER || payload > arena.len() {
            return;
        }
        let block = read_u64(arena, payload - 8) as usize;
        if block + HEADER > arena.len() {
            return;
        }
        let size = block_size(arena, block);

        // Find the insertion point that keeps the list address-ordered.
        let mut prev: Option<usize> = None;
        let mut cursor = self.head;
        while let Some(candidate) = cursor {
            if candidate > block {
                break;
            }
            prev = Some(candidate);
            cursor = next_free(arena, candidate);
        }

        set_block_size(arena, block, size);
        set_next_free(arena, block, cursor);
        self.relink(arena, prev, Some(block));

        // Merge forward first: merging backward would move the block we are
        // holding, and the successor offset would no longer be reachable.
        if let Some(next) = cursor
            && block + size == next
        {
            let merged = size + block_size(arena, next);
            set_block_size(arena, block, merged);
            set_next_free(arena, block, next_free(arena, next));
        }

        if let Some(previous) = prev {
            let previous_size = block_size(arena, previous);
            if previous + previous_size == block {
                let merged = previous_size + block_size(arena, block);
                set_block_size(arena, previous, merged);
                set_next_free(arena, previous, next_free(arena, block));
            }
        }
    }

    /// Point `prev`'s next pointer (or the head) at `target`.
    fn relink(&mut self, arena: &mut [u8], prev: Option<usize>, target: Option<usize>) {
        match prev {
            Some(block) => set_next_free(arena, block, target),
            None => self.head = target,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARENA: usize = 4096;

    fn arena() -> (Vec<u8>, FreeList) {
        let mut buf = vec![0u8; ARENA];
        let list = FreeList::new(&mut buf).unwrap();
        (buf, list)
    }

    #[test]
    fn a_fresh_arena_is_one_block() {
        let (buf, list) = arena();
        assert_eq!(list.free_blocks(&buf), 1);
        assert_eq!(list.free_bytes(&buf), ARENA);
    }

    #[test]
    fn too_small_an_arena_is_rejected() {
        let mut tiny = vec![0u8; HEADER];
        assert!(FreeList::new(&mut tiny).is_none());
    }

    #[test]
    fn allocations_do_not_overlap() {
        let (mut buf, mut list) = arena();
        let a = list.alloc(&mut buf, 64, 16).unwrap();
        let b = list.alloc(&mut buf, 64, 16).unwrap();
        assert!(a + 64 <= b || b + 64 <= a, "a={a} b={b} overlap");
    }

    #[test]
    fn alignment_is_honoured() {
        let (mut buf, mut list) = arena();
        // Push the cursor off a large alignment first.
        list.alloc(&mut buf, 24, 16).unwrap();
        for align in [16usize, 32, 64, 256] {
            let p = list.alloc(&mut buf, 8, align).unwrap();
            assert_eq!(p % align, 0, "align {align} not honoured for {p}");
        }
    }

    #[test]
    fn a_bad_alignment_or_empty_request_is_rejected() {
        let (mut buf, mut list) = arena();
        assert_eq!(list.alloc(&mut buf, 8, 24), None);
        assert_eq!(list.alloc(&mut buf, 0, 16), None);
    }

    #[test]
    fn exhaustion_returns_none() {
        let (mut buf, mut list) = arena();
        assert!(list.alloc(&mut buf, ARENA * 2, 16).is_none());
    }

    /// The point of having a free list at all: memory comes back.
    #[test]
    fn freed_memory_is_reused() {
        let (mut buf, mut list) = arena();
        let before = list.free_bytes(&buf);

        let p = list.alloc(&mut buf, 128, 16).unwrap();
        assert!(list.free_bytes(&buf) < before);
        list.dealloc(&mut buf, p);
        assert_eq!(list.free_bytes(&buf), before, "all bytes must return");

        let q = list.alloc(&mut buf, 128, 16).unwrap();
        assert_eq!(p, q, "the same block should be handed out again");
    }

    /// Without coalescing, alternating alloc/free shreds the arena into
    /// fragments that no large request can ever use again.
    #[test]
    fn adjacent_frees_coalesce_back_into_one_block() {
        let (mut buf, mut list) = arena();
        let a = list.alloc(&mut buf, 64, 16).unwrap();
        let b = list.alloc(&mut buf, 64, 16).unwrap();
        let c = list.alloc(&mut buf, 64, 16).unwrap();

        // Free out of order to exercise both merge directions.
        list.dealloc(&mut buf, b);
        list.dealloc(&mut buf, a);
        list.dealloc(&mut buf, c);

        assert_eq!(list.free_blocks(&buf), 1, "arena must be whole again");
        assert_eq!(list.free_bytes(&buf), ARENA);
    }

    #[test]
    fn a_hole_is_reused_before_the_tail() {
        let (mut buf, mut list) = arena();
        let a = list.alloc(&mut buf, 64, 16).unwrap();
        let _b = list.alloc(&mut buf, 64, 16).unwrap();
        list.dealloc(&mut buf, a);

        // First fit must take the hole left by `a`, not grow into fresh space.
        let c = list.alloc(&mut buf, 32, 16).unwrap();
        assert_eq!(c, a);
    }

    /// A long random-ish workload must never hand out overlapping memory and
    /// must return every byte once everything is freed.
    #[test]
    fn a_churn_workload_stays_consistent_and_fully_reclaims() {
        let (mut buf, mut list) = arena();
        let total = list.free_bytes(&buf);
        let mut live: Vec<(usize, usize)> = Vec::new();

        // Deterministic pseudo-random sizes: reproducible failures matter more
        // than statistical coverage here.
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };

        for round in 0..2000 {
            let free_it = !live.is_empty() && (round % 3 == 0 || live.len() > 12);
            if free_it {
                let index = (next() as usize) % live.len();
                let (offset, _) = live.swap_remove(index);
                list.dealloc(&mut buf, offset);
                continue;
            }

            let size = 1 + (next() as usize % 200);
            let align = 1 << (next() as usize % 6); // 1..32
            let align = align.max(1);
            if let Some(offset) = list.alloc(&mut buf, size, align) {
                assert_eq!(offset % align, 0);
                // Fill with a marker and check nothing else was clobbered.
                for (a, b) in live.iter().map(|&(o, s)| (o, o + s)) {
                    assert!(
                        offset + size <= a || b <= offset,
                        "round {round}: [{offset},{}) overlaps [{a},{b})",
                        offset + size
                    );
                }
                live.push((offset, size));
            }
        }

        for (offset, _) in live {
            list.dealloc(&mut buf, offset);
        }
        assert_eq!(list.free_blocks(&buf), 1, "churn left the arena fragmented");
        assert_eq!(list.free_bytes(&buf), total, "churn leaked bytes");
    }
}
