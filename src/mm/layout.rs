//! Kernel memory layout: what the linker produced, and how it must be mapped.
//!
//! Each region gets the weakest permissions that still let it do its job. The
//! guard page between the page-table arena and the stack is deliberately
//! absent from this list: an unmapped page is the whole mechanism.

use kernel_core::paging::{MemKind, Perms};

use crate::arch::mmu::Region;
use crate::bsp::board::memmap;

unsafe extern "C" {
    static __image_start: u8;
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
    static __pagetables_start: u8;
    static __pagetables_end: u8;
    static __stack_bottom: u8;
    static __stack_top: u8;
    static __heap_start: u8;
}

/// Number of regions [`kernel_regions`] produces.
pub const REGION_COUNT: usize = 7 + memmap::DEVICE_REGIONS.len();

fn addr(symbol: &u8) -> u64 {
    core::ptr::from_ref(symbol) as u64
}

/// The regions the kernel needs mapped, in ascending address order.
///
/// `heap_end` is exclusive and must be page aligned.
pub fn kernel_regions(heap_end: u64) -> [Region; REGION_COUNT] {
    // SAFETY: linker-provided symbols; only their addresses are taken.
    let (image_start, text_start, text_end) =
        unsafe { (addr(&__image_start), addr(&__text_start), addr(&__text_end)) };
    // SAFETY: as above.
    let (rodata_start, rodata_end, data_start, data_end) = unsafe {
        (
            addr(&__rodata_start),
            addr(&__rodata_end),
            addr(&__data_start),
            addr(&__data_end),
        )
    };
    // SAFETY: as above.
    let (pt_start, pt_end, stack_bottom, stack_top, heap_start) = unsafe {
        (
            addr(&__pagetables_start),
            addr(&__pagetables_end),
            addr(&__stack_bottom),
            addr(&__stack_top),
            addr(&__heap_start),
        )
    };

    let mut regions = [Region {
        base: 0,
        len: 0,
        kind: MemKind::NormalWb,
        perms: Perms::RW,
        name: "unused",
    }; REGION_COUNT];

    // RAM below the kernel image: firmware mailboxes and the secondary-core
    // spin table live here. Mapped so a stray read faults cleanly rather than
    // aborting on an unmapped address, never executable.
    regions[0] = Region {
        base: 0,
        len: image_start,
        kind: MemKind::NormalWb,
        perms: Perms::RW,
        name: "low RAM",
    };

    // The three image regions are what W^X is about: code is executable and
    // read-only, and nothing that is writable is executable.
    regions[1] = Region {
        base: text_start,
        len: text_end - text_start,
        kind: MemKind::NormalWb,
        perms: Perms::RX,
        name: ".text",
    };
    regions[2] = Region {
        base: rodata_start,
        len: rodata_end - rodata_start,
        kind: MemKind::NormalWb,
        perms: Perms::RO,
        name: ".rodata",
    };
    regions[3] = Region {
        base: data_start,
        len: data_end - data_start,
        kind: MemKind::NormalWb,
        perms: Perms::RW,
        name: ".data/.bss",
    };

    regions[4] = Region {
        base: pt_start,
        len: pt_end - pt_start,
        kind: MemKind::NormalWb,
        perms: Perms::RW,
        name: "page tables",
    };

    // Gap: the guard page sits between the arena and the stack and is left
    // unmapped, so a stack overflow takes a translation fault.
    regions[5] = Region {
        base: stack_bottom,
        len: stack_top - stack_bottom,
        kind: MemKind::NormalWb,
        perms: Perms::RW,
        name: "stack",
    };

    regions[6] = Region {
        base: heap_start,
        len: heap_end.saturating_sub(heap_start),
        kind: MemKind::NormalWb,
        perms: Perms::RW,
        name: "heap",
    };

    for (slot, &(base, len, name)) in regions[7..].iter_mut().zip(&memmap::DEVICE_REGIONS) {
        *slot = Region {
            base: base as u64,
            len: len as u64,
            kind: MemKind::Device,
            perms: Perms::RW,
            name,
        };
    }

    regions
}

/// Address of the guard page, for diagnostics and for the fault self-check.
pub fn guard_page() -> u64 {
    unsafe extern "C" {
        static __guard_start: u8;
    }
    // SAFETY: linker-provided symbol; only its address is taken.
    unsafe { addr(&__guard_start) }
}
