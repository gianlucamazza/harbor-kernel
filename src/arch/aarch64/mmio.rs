//! Memory-mapped I/O primitives for AArch64.
//!
//! All device register access goes through these helpers, so *volatility* has
//! one definition: no compiler is free to merge, reorder or elide a device
//! access.
//!
//! **Ordering is not defined here, and deliberately so.** The peripheral window
//! is mapped Device-nGnRnE, which already orders device accesses against each
//! other in program order — that is what the mapping buys, and it is why no
//! barrier appears below. What it does *not* order is a device access against
//! normal memory, and that pairing is specific to the device: `drivers::gicv2`
//! issues its own `dsb` after writing `EOIR` because the interrupt it just
//! retired has to be visible before the handler's bookkeeping lands. A barrier
//! placed here would be right for that case and wrong, or merely wasteful, for
//! every other. Callers that need one write it, next to the reason.

use core::ptr::{read_volatile, write_volatile};

/// Device register window rooted at a physical MMIO base address.
#[derive(Clone, Copy)]
pub struct Mmio {
    base: usize,
}

impl Mmio {
    /// Create a handle for the register block at `base`.
    ///
    /// # Safety
    ///
    /// `base` must be a valid MMIO region for the lifetime of this handle.
    /// Concurrent access from other cores or DMA must be excluded by the caller
    /// until higher-level synchronisation exists.
    #[inline(always)]
    pub const unsafe fn new(base: usize) -> Self {
        Self { base }
    }

    /// Physical base address of this register window.
    ///
    /// Lets a handle be published through an atomic (an `Option<Mmio>` in a
    /// `static` would be read non-atomically by an IRQ handler).
    #[inline(always)]
    pub const fn base(self) -> usize {
        self.base
    }

    /// Read a 32-bit register at `offset` from the base.
    #[inline(always)]
    pub fn read32(self, offset: usize) -> u32 {
        // SAFETY: caller established a valid MMIO mapping when constructing `self`.
        unsafe { read_volatile((self.base + offset) as *const u32) }
    }

    /// Write a 32-bit register at `offset` from the base.
    #[inline(always)]
    pub fn write32(self, offset: usize, value: u32) {
        // SAFETY: caller established a valid MMIO mapping when constructing `self`.
        unsafe { write_volatile((self.base + offset) as *mut u32, value) }
    }
}
