//! Kernel memory management — bump allocator (M2).
//!
//! The address arithmetic lives in [`kernel_core::bump`] and is unit-tested on
//! the host; this module owns the linker symbol, the single instance, and the
//! conversion to raw pointers.

use kernel_core::bump::Bump;

use crate::bsp::board::memmap::IDENTITY_RAM_END;
use crate::sync::SyncCell;

// Linker symbol (physical = virtual after identity map).
unsafe extern "C" {
    static __heap_start: u8;
}

/// First address available to the heap, from the linker script.
pub fn heap_start() -> usize {
    core::ptr::addr_of!(__heap_start) as usize
}

/// The kernel heap.
///
/// Single core through M2: every accessor below runs either during bootstrap
/// with IRQs masked, or from the main loop. [`SyncCell`] states that invariant
/// in the type instead of leaving it to a comment on a `static mut`.
static HEAP: SyncCell<Bump> = SyncCell::new(Bump::empty());

/// Initialise the bump heap from `__heap_start` up to `end` (exclusive).
///
/// Returns `false` if the region is empty, leaving the heap unusable rather
/// than killing the boot.
///
/// # Safety
/// Single core; call once after the MMU identity map covers `[__heap_start, end)`.
pub unsafe fn init_heap(end: usize) -> bool {
    unsafe {
        let end = end.min(IDENTITY_RAM_END);
        match Bump::new(heap_start(), end) {
            Some(bump) => {
                *HEAP.get() = bump;
                true
            }
            None => false,
        }
    }
}

/// Bytes remaining in the bump heap.
pub fn heap_remaining() -> usize {
    // SAFETY: single core; no concurrent accessor.
    unsafe { (*HEAP.get()).remaining() }
}

/// Bump-allocate `size` bytes aligned to `align` (power of two).
///
/// Returns a writable pointer, or `None` if exhausted or `align` is invalid.
pub fn alloc(size: usize, align: usize) -> Option<*mut u8> {
    // SAFETY: single core; no concurrent accessor.
    let heap = unsafe { &mut *HEAP.get() };
    heap.alloc(size, align).map(|addr| addr as *mut u8)
}

/// Convenience: allocate and zero `size` bytes.
pub fn alloc_zeroed(size: usize, align: usize) -> Option<*mut u8> {
    let p = alloc(size, align)?;
    // SAFETY: freshly bump-allocated region of `size` bytes.
    unsafe {
        core::ptr::write_bytes(p, 0, size);
    }
    Some(p)
}
