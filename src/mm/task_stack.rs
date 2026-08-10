//! Heap-allocated stacks with an optional unmapped guard page (ADR-0006 / K5-S).
//!
//! Layout for **Full** / **Thin** (addresses grow up, stacks grow down):
//!
//! ```text
//!   base ──► [ guard page — unmapped ][ usable pages — mapped RW ]
//!            low                     high = initial SP
//! ```
//!
//! **Mini** (ADR-0086): one mapped page only — no unmapped guard. Short EL1
//! workers only; overflow is not a translation fault into a hole.

use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::layout::{GuardedStack, LayoutError, validate_guarded_stack};
use kernel_core::paging::PAGE_SIZE;

use crate::arch::mmu;
use crate::mm;

/// Stacks leaked because their guard page could not be remapped. See
/// [`abandoned_stacks`].
static ABANDONED: AtomicU32 = AtomicU32::new(0);

/// Task stacks leaked rather than freed with an unmapped guard page.
///
/// Non-zero means the heap has permanently lost memory *and* that
/// [`mmu::map`] refused a page this kernel believed it owned — the second is
/// the more interesting half.
pub fn abandoned_stacks() -> u32 {
    ABANDONED.load(Ordering::Relaxed)
}

/// Why a task stack could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackError {
    /// Usable size was zero or not a multiple of the page size.
    BadSize,
    /// The heap could not satisfy a page-aligned allocation.
    Oom,
    /// Geometry failed [`validate_guarded_stack`].
    Layout(LayoutError),
    /// [`mmu::unmap`] of the guard page failed.
    Unmap(mmu::MmuError),
}

/// One task stack: usable range (+ optional guard), owned until [`TaskStack::release`].
///
/// `base` is stored as `usize` so the TCB table can live in a `Mutex`
/// (`*mut u8` is not `Send`). The pointer is only formed at alloc/release.
pub struct TaskStack {
    /// Allocation base (guard page when present; else stack page).
    base: usize,
    /// Usable high address (initial SP).
    stack_top: usize,
    /// Present when an unmapped guard page was carved out.
    geometry: Option<GuardedStack>,
}

impl TaskStack {
    /// Allocate `usable_bytes` of stack (page multiple) plus one guard page.
    ///
    /// The guard is unmapped before this returns. Physical memory for the guard
    /// stays part of the allocation so the free-list never sees a virtual hole.
    pub fn allocate(usable_bytes: usize) -> Result<Self, StackError> {
        if usable_bytes == 0 || !usable_bytes.is_multiple_of(PAGE_SIZE as usize) {
            return Err(StackError::BadSize);
        }
        let total = usable_bytes
            .checked_add(PAGE_SIZE as usize)
            .ok_or(StackError::BadSize)?;

        let base = mm::alloc(total, PAGE_SIZE as usize).ok_or(StackError::Oom)?;
        let base_u = base as u64;
        let geometry = GuardedStack {
            guard: (base_u, base_u + PAGE_SIZE),
            stack: (base_u + PAGE_SIZE, base_u + total as u64),
            name: "task stack",
        };
        validate_guarded_stack(&geometry).map_err(StackError::Layout)?;

        // SAFETY: the range is this allocation's own first page, validated by
        // `validate_guarded_stack` above and not yet handed to any task, so no
        // live SP points into it. `mmu::unmap` serialises under MAP_LOCK
        // (ADR-0077 / F-R1-P1).
        let unmap_err = unsafe { mmu::unmap(geometry.guard.0, PAGE_SIZE) };
        if let Err(error) = unmap_err {
            // Remap is not needed — still fully mapped. Return the pages.
            // SAFETY: allocation still fully mapped; we never unmapped.
            unsafe { mm::dealloc(base) };
            return Err(StackError::Unmap(error));
        }

        Ok(Self {
            base: base as usize,
            stack_top: geometry.stack.1 as usize,
            geometry: Some(geometry),
        })
    }

    /// Mini stack (ADR-0086 / K5-S): **one** mapped page, no unmapped guard.
    ///
    /// Heap cost is half of Thin (4 KiB vs 8 KiB). Overflow is not fenced by a
    /// hole — only for short EL1 workers that yield/exit.
    pub fn allocate_mini() -> Result<Self, StackError> {
        let len = PAGE_SIZE as usize;
        let base = mm::alloc(len, len).ok_or(StackError::Oom)?;
        let base_u = base as u64;
        Ok(Self {
            base: base as usize,
            stack_top: (base_u + len as u64) as usize,
            geometry: None,
        })
    }

    /// Initial stack pointer: top of the usable region (AAPCS full-descending).
    #[inline]
    pub fn initial_sp(&self) -> usize {
        self.stack_top
    }

    /// The unmapped guard page, `[low, high)`, if this stack has one.
    #[cfg_attr(not(feature = "bringup"), allow(dead_code))]
    #[inline]
    pub fn guard_range(&self) -> Option<(u64, u64)> {
        self.geometry.map(|g| g.guard)
    }

    /// The usable stack, `[low, high)`. High is the initial SP.
    #[cfg_attr(not(feature = "bringup"), allow(dead_code))]
    #[inline]
    pub fn stack_range(&self) -> (u64, u64) {
        match self.geometry {
            Some(g) => g.stack,
            None => (self.base as u64, self.stack_top as u64),
        }
    }

    /// Remap any guard and return the allocation to the heap.
    ///
    /// # Safety
    /// No code may still be executing on this stack.
    pub unsafe fn release(self) {
        if let Some(geometry) = self.geometry {
            use kernel_core::layout::Region;
            use kernel_core::paging::{MemKind, Perms};

            let region = Region {
                base: geometry.guard.0,
                len: PAGE_SIZE,
                kind: MemKind::NormalWb,
                perms: Perms::RW,
                name: "task stack guard restore",
            };
            // SAFETY: restores the mapping this stack itself removed, over the
            // guard page of an allocation the caller guarantees is dead (see
            // the fn contract). The physical pages never left the allocation.
            let remapped = unsafe { mmu::map(&region) };

            if remapped.is_err() {
                ABANDONED.fetch_add(1, Ordering::Relaxed);
                core::mem::forget(self);
                return;
            }
        }

        let base = self.base;
        // SAFETY: pages mapped again (or Mini never unmapped); no live SP.
        unsafe { mm::dealloc(base as *mut u8) };
        core::mem::forget(self);
    }
}

impl Drop for TaskStack {
    fn drop(&mut self) {
        // Leak rather than free with a live unmapped guard: releasing needs an
        // explicit `release` when the task has exited. Dropping without that is
        // a bug; leaking keeps the heap consistent.
    }
}
