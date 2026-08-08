//! Kernel memory management.
//!
//! Two allocators with different jobs:
//!
//! - [`kernel_core::bump`] — early boot, before the heap region is mapped.
//!   Cannot free, and does not need to: nothing allocated that early is
//!   returned.
//! - [`kernel_core::heap::FreeList`] — the general allocator, wired to
//!   `GlobalAlloc` so `alloc::boxed::Box` and `alloc::vec::Vec` work. Tasks,
//!   mailboxes and message queues all need memory with a lifetime.
//!
//! The allocator arithmetic is unit-tested on the host; this module owns the
//! linker symbol, the single instance, the interrupt discipline, and the
//! conversion between offsets and raw pointers.

pub mod asid;
pub mod aspace;
pub mod early;
pub mod frames;
pub mod layout;
pub mod task_stack;

pub use aspace::{AddressSpace, AsError};
pub use task_stack::{StackError, TaskStack};

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::heap::FreeList;

use crate::arch::cpu;
use crate::bsp::board::memmap::IDENTITY_RAM_END;
use crate::sync::SyncCell;

// Linker symbol (physical = virtual after identity map).
unsafe extern "C" {
    static __heap_start: u8;
}

/// First address available to the heap, from the linker script.
pub fn heap_start() -> usize {
    &raw const __heap_start as usize
}

/// The kernel heap: free list plus the arena it indexes.
struct KernelHeap {
    list: Option<FreeList>,
    base: usize,
    len: usize,
}

impl KernelHeap {
    const fn uninitialised() -> Self {
        Self {
            list: None,
            base: 0,
            len: 0,
        }
    }

    /// The arena as a slice.
    ///
    /// # Safety
    /// The heap must be initialised and the region mapped writable. Only one
    /// such slice may exist at a time — guaranteed by every caller running
    /// inside `cpu::without_irqs`.
    unsafe fn arena(&mut self) -> &mut [u8] {
        // SAFETY: `base` and `len` describe the mapped heap region, and taking
        // `&mut self` is what makes the "one slice at a time" half of the
        // contract enforceable rather than merely stated — a second slice would
        // need a second `&mut` to this allocator, which the borrow checker
        // refuses. The mapping and the masking remain the caller's.
        unsafe { core::slice::from_raw_parts_mut(self.base as *mut u8, self.len) }
    }
}

/// The single kernel heap.
///
/// Accessed from the main loop and, once M3 arrives, from IRQ context. Every
/// accessor takes the critical section rather than assuming single-core
/// exclusivity, because an allocator interrupted mid-splice corrupts its own
/// free list and the damage surfaces arbitrarily later.
static HEAP: SyncCell<KernelHeap> = SyncCell::new(KernelHeap::uninitialised());

/// Initialise the kernel heap from `__heap_start` up to `end` (exclusive).
///
/// Returns `false` if the region is too small, leaving the heap unusable
/// rather than killing the boot.
///
/// # Safety
/// Call once, after the MMU maps `[__heap_start, end)` as writable RAM.
pub unsafe fn init_heap(end: usize) -> bool {
    let start = heap_start();
    let end = end.min(IDENTITY_RAM_END);
    if end <= start {
        return false;
    }

    cpu::without_irqs(|| {
        // SAFETY: the caller established that this region is mapped writable
        // and that no other accessor exists yet.
        unsafe {
            let heap = &mut *HEAP.get();
            heap.base = start;
            heap.len = end - start;
            heap.list = FreeList::new(heap.arena());
            heap.list.is_some()
        }
    })
}

/// Run `f` against the heap with interrupts masked.
///
/// `f` receives the arena base as well, so callers that need to turn an offset
/// into a pointer can do it inside the same critical section instead of taking
/// a second one — two sections in a row read as a protection the code does not
/// actually provide.
fn with_heap<R>(f: impl FnOnce(&mut FreeList, &mut [u8], usize) -> R) -> Option<R> {
    cpu::without_irqs(|| {
        // SAFETY: interrupts are masked and this is the only accessor path, so
        // no second `&mut` to the heap or its arena can exist.
        unsafe {
            let heap = &mut *HEAP.get();
            let mut list = heap.list?;
            let base = heap.base;
            let result = f(&mut list, heap.arena(), base);
            heap.list = Some(list);
            Some(result)
        }
    })
}

/// Bytes still free in the kernel heap.
pub fn heap_remaining() -> usize {
    with_heap(|list, arena, _| list.free_bytes(arena)).unwrap_or(0)
}

/// Free blocks in the kernel heap — rising counts mean fragmentation.
pub fn heap_fragments() -> usize {
    with_heap(|list, arena, _| list.free_blocks(arena)).unwrap_or(0)
}

/// Allocate `size` bytes aligned to `align` (power of two).
pub fn alloc(size: usize, align: usize) -> Option<*mut u8> {
    with_heap(|list, arena, base| {
        let offset = list.alloc(arena, size, align)?;
        Some((base + offset) as *mut u8)
    })?
}

/// Frees the allocator refused, since boot.
///
/// A double free is a bug in the code that owns the pointer, and the allocator
/// is the only place that can see it. Refusing keeps the heap intact — the
/// memory leaks instead — but a refusal nobody counts is a bug nobody learns
/// about, so the number is reported by the console loop and asserted on by
/// the boot check.
static REFUSED_FREES: AtomicU32 = AtomicU32::new(0);

/// How many frees have been refused as invalid.
pub fn refused_frees() -> u32 {
    REFUSED_FREES.load(Ordering::Relaxed)
}

/// Return memory from [`alloc`].
///
/// # Safety
/// `ptr` must have come from [`alloc`] on this heap and not been freed since.
/// Violating this is detected rather than trusted: see [`refused_frees`].
pub unsafe fn dealloc(ptr: *mut u8) {
    let addr = ptr as usize;
    let refused = with_heap(|list, arena, base| {
        // A pointer below the arena cannot be ours, and neither can one the
        // allocator refuses. Both are counted: the alternative is a heap that
        // quietly leaks whenever a caller is wrong about what it owns.
        match addr.checked_sub(base) {
            Some(offset) => list.dealloc(arena, offset).is_err(),
            None => true,
        }
    });

    if refused.unwrap_or(false) {
        REFUSED_FREES.fetch_add(1, Ordering::Relaxed);
    }
}

// There is no `alloc_zeroed` helper here on purpose: with a `GlobalAlloc`
// installed, `alloc::alloc::alloc_zeroed` and the collections built on it are
// the way to ask for zeroed memory.

/// Adapter making the kernel heap the Rust global allocator.
struct KernelAllocator;

// SAFETY: `alloc`/`dealloc` serialise on the interrupt-masked critical section
// and hand out non-overlapping regions of the mapped heap.
unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        alloc(layout.size(), layout.align()).unwrap_or(core::ptr::null_mut())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // SAFETY: forwarded from the caller's obligation.
        unsafe { dealloc(ptr) }
    }
}

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;
