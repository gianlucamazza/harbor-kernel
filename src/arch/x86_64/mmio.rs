//! MMIO and port I/O for x86_64 lab.
//!
//! L0 console uses **port I/O** (`inb`/`outb`). Memory-mapped `Mmio` is the
//! arch-contract role for later APIC/device windows (progressive-isa).

#![allow(dead_code)] // Mmio window unused on L0 COM1 path; port I/O is used

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};

/// Volatile MMIO window (memory-mapped).
pub struct Mmio {
    base: usize,
}

impl Mmio {
    /// # Safety
    /// `base` must be a valid device mapping for the kernel's lifetime.
    pub const unsafe fn new(base: usize) -> Self {
        Self { base }
    }

    #[inline]
    pub fn read32(&self, offset: usize) -> u32 {
        // SAFETY: caller contracted the base; offset is device-specific.
        unsafe { read_volatile((self.base + offset) as *const u32) }
    }

    #[inline]
    pub fn write32(&self, offset: usize, value: u32) {
        // SAFETY: as read32.
        unsafe { write_volatile((self.base + offset) as *mut u32, value) }
    }
}

/// Write a byte to an I/O port.
///
/// # Safety
/// `port` must be a valid port the firmware/QEMU exposes.
#[inline]
pub unsafe fn outb(port: u16, val: u8) {
    // SAFETY: port I/O; caller names a real port.
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") val, options(nostack, nomem, preserves_flags));
    }
}

/// Read a byte from an I/O port.
///
/// # Safety
/// `port` must be a valid port.
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    // SAFETY: port I/O.
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") val, options(nostack, nomem, preserves_flags));
    }
    val
}
