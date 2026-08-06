//! The identity map the kernel runs under before anything else exists.
//!
//! A coarse map of 1 GiB blocks, built entirely at compile time and enabled
//! from `boot.s` before any other Rust runs. It exists so that *no kernel code
//! ever executes without memory attributes*: with translation off every access
//! is Device-nGnRnE, where the `LDXR`/`STXR` pair behind an atomic
//! read-modify-write does not make progress on Cortex-A72 — an
//! `AtomicBool::swap` in early boot hangs the board silently, and emulators do
//! not reproduce it. Rather than ask every future author to remember that, the
//! window is removed. See [ADR-0003](../../docs/adr/0003-early-mmu.md).
//!
//! # Why it lives in `mm` and not in `arch`
//!
//! This map is the one place where board and CPU meet before anything exists,
//! and it used to be smuggled into `src/arch/aarch64/mmu.rs` — where it encoded
//! "three gigabytes of RAM, then peripherals" for a BCM2711, inside the tree
//! that `architecture.md` rule 3 reserves for CPU and ISA. That was **F23**,
//! the last of the thirty review findings to stay open, and it stayed open
//! because the fix looked impossible: `early_mmu_enable` is called from
//! `boot.s` with `bl` and no arguments, so nothing can be passed in, and
//! `make layering` forbids `arch` from importing `bsp`.
//!
//! The way out is to stop calling it an `arch` concern. `mm` may already see
//! both trees, so the seam gets a name instead of a hiding place: the board
//! says *which gigabyte is what* ([`memmap::EARLY_BLOCKS`]), the architecture
//! says *what a block descriptor is and how to turn translation on*
//! ([`kernel_core::paging`], [`mmu::enable_identity`]), and this module is the
//! only thing that needs to know both.
//!
//! # Why it is `const`
//!
//! The early map has no error path: it is installed before the console exists,
//! so a bad descriptor could not be reported. Evaluating it in `const` context
//! turns that into a compile error instead — which is why [`early_block`]
//! panics rather than returning an error nobody could read.

use kernel_core::paging::{self, Level, MemKind, Perms};

use crate::arch::mmu::{self, Table};
use crate::bsp::board::memmap;

/// Writable and executable — only ever used here, because a 1 GiB granularity
/// cannot distinguish `.text` from the stack. `mmu::activate` replaces this map
/// with a W^X one before any of the kernel's own untrusted input is touched.
const RWX: Perms = Perms {
    write: true,
    execute: true,
    user: false,
};

/// Encode a level-1 block or fail the build.
const fn early_block(pa: u64, kind: MemKind, perms: Perms) -> u64 {
    match paging::leaf(Level::L1, pa, kind, perms) {
        Some(descriptor) => descriptor,
        None => panic!("early identity map: unencodable block address"),
    }
}

/// Size of a level-1 block with the kernel's `T0SZ`. Architectural, not board.
const BLOCK_SIZE: u64 = 1 << 30;

/// The early identity map, resolved at compile time.
///
/// Not `mut` and never written at runtime: the table lives in `.rodata`, and
/// the page-table walker only reads it (the access flag is pre-set, so there is
/// no hardware update either).
static EARLY_L1: Table = Table {
    entries: {
        let mut entries = [0u64; paging::ENTRIES_PER_TABLE];
        let mut i = 0;
        while i < memmap::EARLY_BLOCKS.len() {
            let (kind, perms) = match memmap::EARLY_BLOCKS[i] {
                MemKind::Device => (MemKind::Device, Perms::RW),
                kind => (kind, RWX),
            };
            entries[i] = early_block(i as u64 * BLOCK_SIZE, kind, perms);
            i += 1;
        }
        entries
    },
};

/// Enable translation with the compile-time identity map.
///
/// Called from `_start` once the stack exists and BSS is clear, before
/// `kernel_main`. After this returns, memory has attributes and the rest of the
/// kernel — atomics included — behaves as the architecture documents.
///
/// `#[unsafe(no_mangle)]` because `boot.s` branches to it by name;
/// `scripts/check-pre-mmu-path.sh` walks `_start` to here and refuses anything
/// on the way that could touch memory without attributes. Which module the
/// symbol lives in does not change either fact.
///
/// # Safety
/// Call exactly once, on the primary core, with interrupts masked and
/// translation off.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn early_mmu_enable() {
    // SAFETY: the caller guarantees translation is off and this runs once. The
    // table is a `static` in `.rodata` that outlives every use, so the address
    // handed to the walker stays valid for the life of the kernel.
    unsafe { mmu::enable_identity(&raw const EARLY_L1 as u64) }
}
