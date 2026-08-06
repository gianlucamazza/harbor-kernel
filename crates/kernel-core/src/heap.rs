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

/// Marks a block as handed out, in the low bit of its size word.
///
/// Sizes are multiples of [`GRAIN`], so the low four bits of that word are
/// always zero and cost nothing to use. This is what lets `dealloc` tell a live
/// block from one already on the free list, which is the whole of the
/// double-free defence.
const ALLOCATED: u64 = 1;

/// Why a [`FreeList::dealloc`] was refused.
///
/// Refused, not ignored: in every case the free list is left exactly as it was,
/// so a bad free leaks rather than corrupting the heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeError {
    /// The payload offset, or the block header it points at, lies outside the
    /// arena — the pointer did not come from this allocator.
    OutOfBounds,
    /// The block header is not one this allocator could have written:
    /// misaligned, or inconsistent with the payload it claims to own.
    Corrupt,
    /// The block is not marked as allocated. Either it has already been freed,
    /// or the pointer was never handed out.
    NotAllocated,
}

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
    (read_u64(arena, block) & !ALLOCATED) as usize
}

/// Write a size and clear the allocated mark: every caller of this is either
/// creating a free block or returning one to the list.
fn set_block_size(arena: &mut [u8], block: usize, size: usize) {
    write_u64(arena, block, size as u64);
}

fn is_allocated(arena: &[u8], block: usize) -> bool {
    read_u64(arena, block) & ALLOCATED != 0
}

fn mark_allocated(arena: &mut [u8], block: usize) {
    let word = read_u64(arena, block);
    write_u64(arena, block, word | ALLOCATED);
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
                // Mark it live. `dealloc` refuses any block without this, which
                // is what makes a double free a rejection instead of a second
                // insertion into an address-ordered list — that would build a
                // cycle and hand the same memory out twice.
                mark_allocated(arena, block);
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
    ///
    /// # Refusing bad frees
    ///
    /// A free that cannot be justified is refused and the list left untouched.
    /// This matters most for the double free: the list is ordered by address,
    /// so inserting the same block twice links it to itself, and from then on
    /// the allocator hands the same memory to two owners. Silently. The
    /// allocated mark makes the second attempt an error instead.
    ///
    /// The check is sound in the direction that matters — a legitimate free is
    /// never refused. The converse does not hold: metadata lives in the arena
    /// it manages, so a wild pointer into memory that happens to look like a
    /// live header can still be accepted. Detecting every bad free needs
    /// out-of-band metadata, which costs memory this kernel would rather spend
    /// elsewhere.
    pub fn dealloc(&mut self, arena: &mut [u8], payload: usize) -> Result<(), FreeError> {
        if payload < HEADER || payload > arena.len() {
            return Err(FreeError::OutOfBounds);
        }
        // These eight bytes are the block's back-pointer while it is allocated,
        // and its next-free pointer once it is not — the two uses overlap
        // whenever alignment leaves the payload at `block + HEADER`. So the
        // sentinel appearing here is itself evidence: the block is already on
        // the free list, and this is a second free of it.
        let recorded = read_u64(arena, payload - 8);
        if recorded == NIL {
            return Err(FreeError::NotAllocated);
        }
        let block = recorded as usize;

        // Arithmetic on a value read out of the arena has to be checked: a
        // corrupt header is exactly the case this function exists to survive.
        let Some(header_end) = block.checked_add(HEADER) else {
            return Err(FreeError::OutOfBounds);
        };
        if header_end > arena.len() {
            return Err(FreeError::OutOfBounds);
        }
        // Every block this allocator creates starts on a `GRAIN` boundary and
        // begins at least `HEADER` bytes before its own payload.
        if !block.is_multiple_of(GRAIN) || header_end > payload {
            return Err(FreeError::Corrupt);
        }
        if !is_allocated(arena, block) {
            return Err(FreeError::NotAllocated);
        }
        let size = block_size(arena, block);
        let Some(block_end) = block.checked_add(size) else {
            return Err(FreeError::Corrupt);
        };
        if size < MIN_BLOCK || block_end > arena.len() || payload >= block_end {
            return Err(FreeError::Corrupt);
        }

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

        Ok(())
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
        list.dealloc(&mut buf, p)
            .expect("a legitimate free must be accepted");
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
        list.dealloc(&mut buf, b)
            .expect("a legitimate free must be accepted");
        list.dealloc(&mut buf, a)
            .expect("a legitimate free must be accepted");
        list.dealloc(&mut buf, c)
            .expect("a legitimate free must be accepted");

        assert_eq!(list.free_blocks(&buf), 1, "arena must be whole again");
        assert_eq!(list.free_bytes(&buf), ARENA);
    }

    /// The defect this defence exists for. Freeing twice used to insert the
    /// same offset into an address-ordered list a second time, linking it to
    /// itself; the allocator then handed the same memory to two owners.
    #[test]
    fn a_second_free_of_the_same_block_is_refused() {
        let (mut buf, mut list) = arena();
        let p = list.alloc(&mut buf, 64, 16).unwrap();
        list.dealloc(&mut buf, p).unwrap();

        assert_eq!(
            list.dealloc(&mut buf, p),
            Err(FreeError::NotAllocated),
            "the second free must be refused"
        );

        // Refused means the list is untouched, not merely that nothing crashed.
        assert_eq!(list.free_blocks(&buf), 1);
        assert_eq!(list.free_bytes(&buf), ARENA);

        // And the allocator still works: a cycle would have shown up here as
        // the same offset coming back twice.
        let a = list.alloc(&mut buf, 64, 16).unwrap();
        let b = list.alloc(&mut buf, 64, 16).unwrap();
        assert_ne!(a, b, "two live allocations must not share an address");
    }

    /// The case the sentinel cannot catch, and the mark must.
    ///
    /// With an alignment above `GRAIN` the payload sits beyond `block + HEADER`,
    /// so its back-pointer never overlaps the free-list next pointer: after the
    /// first free those bytes still hold a perfectly valid block offset. Only
    /// the allocated mark distinguishes the second free from the first.
    #[test]
    fn a_second_free_is_refused_when_the_back_pointer_survives_it() {
        let (mut buf, mut list) = arena();
        let p = list.alloc(&mut buf, 64, 64).unwrap();
        let block = read_u64(&buf, p - 8) as usize;
        list.dealloc(&mut buf, p).unwrap();

        assert_eq!(
            read_u64(&buf, p - 8) as usize,
            block,
            "this test is pointless unless the back-pointer really survived"
        );
        assert_eq!(list.dealloc(&mut buf, p), Err(FreeError::NotAllocated));
        assert_eq!(list.free_bytes(&buf), ARENA);
    }

    /// The third way a second free goes wrong: those bytes now hold a real
    /// offset — the next free block — which is neither the sentinel nor this
    /// block. Refused as `Corrupt`, because a block cannot start after the
    /// payload it claims to own. Without that check the free tail would be
    /// inserted into the list a second time.
    #[test]
    fn a_second_free_is_refused_when_the_next_pointer_points_forward() {
        let (mut buf, mut list) = arena();
        let _a = list.alloc(&mut buf, 64, 16).unwrap();
        let b = list.alloc(&mut buf, 64, 16).unwrap();
        let _c = list.alloc(&mut buf, 64, 16).unwrap();
        list.dealloc(&mut buf, b).unwrap();

        let recorded = read_u64(&buf, b - 8) as usize;
        assert_ne!(recorded as u64, NIL, "not the sentinel case");
        assert!(recorded > b, "the recorded offset points past the payload");
        let before = list.free_bytes(&buf);

        assert_eq!(list.dealloc(&mut buf, b), Err(FreeError::Corrupt));
        assert_eq!(
            list.free_bytes(&buf),
            before,
            "the tail must not be relisted"
        );
        assert_eq!(list.free_blocks(&buf), 2);
    }

    /// A block merged into its predecessor is no longer a block at all. Freeing
    /// it again must still be refused, from the middle of a larger free block.
    #[test]
    fn a_second_free_after_coalescing_is_refused() {
        let (mut buf, mut list) = arena();
        let a = list.alloc(&mut buf, 64, 16).unwrap();
        let b = list.alloc(&mut buf, 64, 16).unwrap();
        list.dealloc(&mut buf, a).unwrap();
        list.dealloc(&mut buf, b).unwrap();
        assert_eq!(list.free_blocks(&buf), 1, "b should have merged into a");

        assert_eq!(list.dealloc(&mut buf, b), Err(FreeError::NotAllocated));
        assert_eq!(list.free_bytes(&buf), ARENA);
    }

    #[test]
    fn a_pointer_this_allocator_never_returned_is_refused() {
        let (mut buf, mut list) = arena();
        let _live = list.alloc(&mut buf, 64, 16).unwrap();
        let before = list.free_bytes(&buf);

        // Past the end, before the first possible payload, and into the middle
        // of the arena where the recorded "block start" is whatever was there.
        assert_eq!(
            list.dealloc(&mut buf, ARENA + 64),
            Err(FreeError::OutOfBounds)
        );
        assert_eq!(list.dealloc(&mut buf, 0), Err(FreeError::OutOfBounds));
        assert!(list.dealloc(&mut buf, ARENA / 2).is_err());

        assert_eq!(list.free_bytes(&buf), before, "nothing may have changed");
    }

    /// The mark lives in the low bits of the size word. If that ever collided
    /// with a real size, every allocation would be one byte short and the
    /// arithmetic above it would drift silently.
    #[test]
    fn the_allocated_mark_does_not_disturb_the_size() {
        let (mut buf, mut list) = arena();
        let p = list.alloc(&mut buf, 100, 16).unwrap();
        let block = read_u64(&buf, p - 8) as usize;

        assert!(is_allocated(&buf, block));
        assert_eq!(block_size(&buf, block) % GRAIN, 0, "size must stay grained");
        assert!(block_size(&buf, block) >= 100 + HEADER);

        list.dealloc(&mut buf, p).unwrap();
        assert!(!is_allocated(&buf, block), "the free must clear the mark");
        assert_eq!(list.free_bytes(&buf), ARENA);
    }

    #[test]
    fn a_hole_is_reused_before_the_tail() {
        let (mut buf, mut list) = arena();
        let a = list.alloc(&mut buf, 64, 16).unwrap();
        let _b = list.alloc(&mut buf, 64, 16).unwrap();
        list.dealloc(&mut buf, a)
            .expect("a legitimate free must be accepted");

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

        // See the note in `ring.rs`: Miri trades volume for scrutiny.
        #[cfg(miri)]
        const ROUNDS: usize = 150;
        #[cfg(not(miri))]
        const ROUNDS: usize = 2000;

        for round in 0..ROUNDS {
            let free_it = !live.is_empty() && (round % 3 == 0 || live.len() > 12);
            if free_it {
                let index = (next() as usize) % live.len();
                let (offset, _) = live.swap_remove(index);
                list.dealloc(&mut buf, offset)
                    .expect("a legitimate free must be accepted");
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
            list.dealloc(&mut buf, offset)
                .expect("a legitimate free must be accepted");
        }
        assert_eq!(list.free_blocks(&buf), 1, "churn left the arena fragmented");
        assert_eq!(list.free_bytes(&buf), total, "churn leaked bytes");
    }
}
