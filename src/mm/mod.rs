//! Kernel memory management — bump allocator (M2).

use crate::arch::mmu::IDENTITY_RAM_END;

// Linker symbol (physical = virtual after identity map).
extern "C" {
    static __heap_start: u8;
}

struct Bump {
    cur: usize,
    end: usize,
}

static mut BUMP: Bump = Bump { cur: 0, end: 0 };

/// Initialise the bump heap from `__heap_start` up to `end` (exclusive).
///
/// # Safety
/// Single core; call once after the MMU identity map covers `[__heap_start, end)`.
pub unsafe fn init_heap(end: usize) {
    let start = core::ptr::addr_of!(__heap_start) as usize;
    let end = end.min(IDENTITY_RAM_END);
    assert!(end > start, "heap end must be above __heap_start");
    BUMP = Bump { cur: start, end };
}

/// Bytes remaining in the bump heap.
pub fn heap_remaining() -> usize {
    // SAFETY: single core.
    unsafe { BUMP.end.saturating_sub(BUMP.cur) }
}

/// Bump-allocate `size` bytes aligned to `align` (power of two).
///
/// Returns a writable pointer, or `None` if exhausted.
pub fn alloc(size: usize, align: usize) -> Option<*mut u8> {
    assert!(align.is_power_of_two());
    // SAFETY: single core.
    let bump = unsafe { &mut *core::ptr::addr_of_mut!(BUMP) };
    if bump.cur == 0 {
        return None; // not initialised
    }
    let aligned = (bump.cur + (align - 1)) & !(align - 1);
    let next = aligned.checked_add(size)?;
    if next > bump.end {
        return None;
    }
    bump.cur = next;
    Some(aligned as *mut u8)
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
