//! Kernel memory layout: read the linker symbols, hand them to the validated
//! builder in [`kernel_core::layout`].
//!
//! Only the symbol reading lives here. Which region gets which permissions and
//! whether the result is coherent — ascending, disjoint, page aligned, never
//! writable *and* executable, guard page covered by nothing — is arithmetic,
//! and is checked by tests that need no board.

use kernel_core::layout::{self, Boundaries, DeviceWindow, GuardedStack, LayoutError, Region};

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
    static __exception_guard_start: u8;
    static __exception_guard_end: u8;
    static __exception_stack_bottom: u8;
    static __exception_stack_top: u8;
    static __guard_start: u8;
    static __guard_end: u8;
    static __stack_bottom: u8;
    static __stack_top: u8;
    static __heap_start: u8;
}

/// Upper bound on the regions the kernel maps: the fixed RAM ones (incl. frame
/// pool) plus the board's device windows.
pub const MAX_REGIONS: usize = 9 + memmap::DEVICE_REGIONS.len();

/// Materialise the address of a linker-provided symbol.
///
/// Declaring these as `static X: u8` claims there is a one-byte object at that
/// address. There is not — there is only an address — and from that claim LLVM
/// correctly derives that two distinct symbols occupy distinct storage. But
/// `__guard_end` and `__stack_bottom` name the *same* address by construction,
/// so `guard_end == stack_bottom` was folded to `false` and the layout
/// validator rejected a perfectly good map. Casting to an integer does not
/// help: the fold happens on the `ptrtoint` operands.
///
/// `core::hint::black_box` suppresses it, and is the wrong tool: its own
/// documentation says the behaviour is unspecified and must not be relied on
/// for correctness. Asking the assembler for the address instead states what
/// is actually meant — *give me the number the linker chose* — and the
/// compiler cannot reason about, or fold, what it cannot see.
macro_rules! symbol_addr {
    ($symbol:ident) => {{
        let address: u64;
        // SAFETY: computes an address from a link-time constant; reads no
        // memory and touches no state.
        unsafe {
            core::arch::asm!(
                "adrp {reg}, {sym}",
                "add  {reg}, {reg}, :lo12:{sym}",
                reg = out(reg) address,
                sym = sym $symbol,
                options(nomem, nostack, preserves_flags),
            );
        }
        address
    }};
}

/// Boundaries as the linker script laid them out, plus named heap / frame pool.
fn boundaries(heap_end: u64, frame_pool_end: u64) -> Boundaries {
    Boundaries {
        image_start: symbol_addr!(__image_start),
        text: (symbol_addr!(__text_start), symbol_addr!(__text_end)),
        rodata: (symbol_addr!(__rodata_start), symbol_addr!(__rodata_end)),
        data: (symbol_addr!(__data_start), symbol_addr!(__data_end)),
        pagetables: (
            symbol_addr!(__pagetables_start),
            symbol_addr!(__pagetables_end),
        ),
        exception_stack: GuardedStack {
            guard: (
                symbol_addr!(__exception_guard_start),
                symbol_addr!(__exception_guard_end),
            ),
            stack: (
                symbol_addr!(__exception_stack_bottom),
                symbol_addr!(__exception_stack_top),
            ),
            name: "exception stack",
        },
        kernel_stack: GuardedStack {
            guard: (symbol_addr!(__guard_start), symbol_addr!(__guard_end)),
            stack: (symbol_addr!(__stack_bottom), symbol_addr!(__stack_top)),
            name: "stack",
        },
        heap: (symbol_addr!(__heap_start), heap_end),
        frame_pool: (heap_end, frame_pool_end),
    }
}

/// Fill `out` with the regions to map, validated.
///
/// `heap_end` / `frame_pool_end` are exclusive and must be page aligned.
/// The frame pool is `[heap_end, frame_pool_end)` (ADR-0012 named carve-out).
pub fn kernel_regions(
    heap_end: u64,
    frame_pool_end: u64,
    out: &mut [Region; MAX_REGIONS],
) -> Result<&mut [Region], LayoutError> {
    let devices = memmap::DEVICE_REGIONS.map(|(base, len, name)| DeviceWindow {
        base: base as u64,
        len: len as u64,
        name,
    });
    layout::kernel_regions(&boundaries(heap_end, frame_pool_end), &devices, out)
}

/// An unmapped `Region` slot, for the caller's buffer.
pub const fn empty_region() -> Region {
    Region {
        base: 0,
        len: 0,
        kind: kernel_core::paging::MemKind::NormalWb,
        perms: kernel_core::paging::Perms::RW,
        name: "unused",
    }
}

/// Address of the guard page, for diagnostics.
pub fn guard_page() -> u64 {
    symbol_addr!(__guard_start)
}
