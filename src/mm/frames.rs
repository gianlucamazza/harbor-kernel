//! Kernel owner of the ADR-0012 physical frame pool.
//!
//! Pure free-list arithmetic lives in [`kernel_core::frame`]. This module
//! binds a **named** phys range (after the bootstrap heap window) to that
//! pool: index → identity-mapped physical address.

use kernel_core::frame::{FrameFreeError, FrameId, FramePool, MAX_FRAMES};

use crate::bsp::board::memmap::{
    FRAME_POOL_BYTES, FRAME_POOL_FRAMES, FRAME_SIZE, IDENTITY_RAM_END,
};
use crate::sync::Mutex;

/// Kernel-side frame pool + phys base of frame 0.
struct Owner {
    pool: FramePool,
    /// Physical address of frame index 0 (identity map).
    base: usize,
    /// Exclusive end of the named region.
    end: usize,
}

impl Owner {
    const fn uninitialised() -> Self {
        Self {
            pool: FramePool::empty(),
            base: 0,
            end: 0,
        }
    }
}

static OWNER: Mutex<Owner> = Mutex::new(Owner::uninitialised());

/// Compute the named frame-pool window immediately after `heap_end`.
///
/// Returns `(base, end)` exclusive end, or `None` if the pool would not fit
/// under [`IDENTITY_RAM_END`] or would be smaller than [`FRAME_POOL_FRAMES`].
pub fn range_after_heap(heap_end: usize) -> Option<(usize, usize)> {
    if !heap_end.is_multiple_of(FRAME_SIZE) {
        return None;
    }
    let base = heap_end;
    let end = base.checked_add(FRAME_POOL_BYTES)?;
    if end > IDENTITY_RAM_END {
        return None;
    }
    // Full named size required — never a silent shrink.
    if end - base != FRAME_POOL_BYTES {
        return None;
    }
    debug_assert_eq!(FRAME_POOL_FRAMES, FRAME_POOL_BYTES / FRAME_SIZE);
    if FRAME_POOL_FRAMES > MAX_FRAMES {
        return None;
    }
    Some((base, end))
}

/// Initialise the pool over a region already mapped as Normal RW.
///
/// # Safety
/// `[base, end)` must be identity-mapped writable RAM, exclusive of the heap
/// and other owners. Call once after the MMU map includes the frame pool.
pub unsafe fn init(base: usize, end: usize) -> bool {
    if end <= base || (end - base) < FRAME_SIZE || !base.is_multiple_of(FRAME_SIZE) {
        return false;
    }
    let n = ((end - base) / FRAME_SIZE).min(MAX_FRAMES) as u32;
    if n == 0 {
        return false;
    }
    // Prefer the full named count when the range matches BSP constants.
    let n = if end - base >= FRAME_POOL_BYTES {
        FRAME_POOL_FRAMES.min(MAX_FRAMES) as u32
    } else {
        n
    };

    OWNER.with(|owner| {
        owner.base = base;
        owner.end = base + (n as usize) * FRAME_SIZE;
        owner.pool = FramePool::new(n);
        true
    })
}

/// Frames still free (0 if uninitialised).
pub fn free_count() -> u32 {
    OWNER.with(|owner| owner.pool.free_count())
}

/// Configured capacity (0 if uninitialised).
pub fn capacity() -> u32 {
    OWNER.with(|owner| owner.pool.capacity())
}

/// Phys base of the pool (0 if uninitialised).
#[expect(dead_code, reason = "S2 AddressSpace / diagnostics")]
pub fn pool_base() -> usize {
    OWNER.with(|owner| owner.base)
}

/// Allocate one frame. Returns `(id, phys)` under the identity map.
pub fn alloc() -> Option<(FrameId, usize)> {
    OWNER.with(|owner| {
        let id = owner.pool.alloc()?;
        let phys = owner.base + (id.index() as usize) * FRAME_SIZE;
        Some((id, phys))
    })
}

/// Free a frame previously returned by [`alloc`].
pub fn free(id: FrameId) -> Result<(), FrameFreeError> {
    OWNER.with(|owner| owner.pool.free(id))
}

/// Physical address of `id` if it lies in this pool's index range.
#[expect(dead_code, reason = "S2 map helpers")]
pub fn phys(id: FrameId) -> Option<usize> {
    OWNER.with(|owner| {
        if id.index() >= owner.pool.capacity() {
            return None;
        }
        Some(owner.base + (id.index() as usize) * FRAME_SIZE)
    })
}
