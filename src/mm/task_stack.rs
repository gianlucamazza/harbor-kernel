//! Heap-allocated stacks with an unmapped guard page (ADR-0006).
//!
//! Layout (addresses grow up, stacks grow down):
//!
//! ```text
//!   base ──► [ guard page — unmapped ][ usable pages — mapped RW ]
//!            low                     high = initial SP
//! ```

use kernel_core::layout::{GuardedStack, LayoutError, validate_guarded_stack};
use kernel_core::paging::PAGE_SIZE;

use crate::arch::cpu;
use crate::arch::mmu;
use crate::mm;

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

/// One task stack: usable range + guard, owned until [`TaskStack::release`].
///
/// `base` is stored as `usize` so the TCB table can live in a `SyncCell`
/// (`*mut u8` is not `Send`). The pointer is only formed at alloc/release.
pub struct TaskStack {
    /// Allocation base (guard page); returned to the heap on release after remap.
    base: usize,
    /// Usable high address (initial SP).
    stack_top: usize,
    geometry: GuardedStack,
}

impl TaskStack {
    /// Allocate `usable_bytes` of stack (page multiple) plus one guard page.
    ///
    /// The guard is unmapped before this returns. Physical memory for the guard
    /// stays part of the allocation so the free-list never sees a virtual hole.
    pub fn allocate(usable_bytes: usize) -> Result<Self, StackError> {
        if usable_bytes == 0 || usable_bytes % PAGE_SIZE as usize != 0 {
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

        let unmap_err = cpu::without_irqs(|| {
            // SAFETY: kernel map active; IRQs masked; guard is one mapped page.
            unsafe { mmu::unmap(geometry.guard.0, PAGE_SIZE) }
        });
        if let Err(error) = unmap_err {
            // Remap is not needed — still fully mapped. Return the pages.
            // SAFETY: allocation still fully mapped; we never unmapped.
            unsafe { mm::dealloc(base) };
            return Err(StackError::Unmap(error));
        }

        Ok(Self {
            base: base as usize,
            stack_top: geometry.stack.1 as usize,
            geometry,
        })
    }

    /// Initial stack pointer: top of the usable region (AAPCS full-descending).
    #[inline]
    pub fn initial_sp(&self) -> usize {
        self.stack_top
    }

    /// Base of the unmapped guard page (for deliberate fault probes).
    ///
    /// Used by `--features bringup` selftest; not referenced in production.
    #[cfg_attr(not(feature = "bringup"), allow(dead_code))]
    #[inline]
    pub fn guard_base(&self) -> u64 {
        self.geometry.guard.0
    }

    /// Remap the guard and return the allocation to the heap.
    ///
    /// # Safety
    /// No code may still be executing on this stack.
    pub unsafe fn release(self) {
        use kernel_core::layout::Region;
        use kernel_core::paging::{MemKind, Perms};

        let region = Region {
            base: self.geometry.guard.0,
            len: PAGE_SIZE,
            kind: MemKind::NormalWb,
            perms: Perms::RW,
            name: "task stack guard restore",
        };
        let _ = cpu::without_irqs(|| {
            // SAFETY: IRQs masked; restoring a page we own before free.
            unsafe { mmu::map(&region) }
        });
        // SAFETY: both pages mapped again; caller guarantees no live SP here.
        unsafe { mm::dealloc(self.base as *mut u8) };
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
