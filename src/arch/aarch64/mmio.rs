//! Memory-mapped I/O primitives for AArch64.
//!
//! All device register access goes through these helpers so volatility and
//! ordering are defined in one place.

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

/// Busy-wait for approximately `cycles` instruction retires.
///
/// Not wall-clock accurate; used only for short hardware settle delays.
#[inline(always)]
pub fn spin_cycles(mut cycles: u32) {
    while cycles > 0 {
        // SAFETY: `nop` has no memory side effects.
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
        cycles -= 1;
    }
}
