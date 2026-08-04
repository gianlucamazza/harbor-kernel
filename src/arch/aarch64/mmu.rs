//! AArch64 stage-1 MMU: two maps, in the order Linux uses.
//!
//! **Early map** ([`early_mmu_enable`]): a coarse identity map of 1 GiB blocks,
//! built entirely at compile time, enabled from `boot.s` before any other Rust
//! runs. It exists so that *no kernel code ever executes without memory
//! attributes*. With translation off every access is Device-nGnRnE, where the
//! `LDXR`/`STXR` pair behind an atomic read-modify-write does not make
//! progress on Cortex-A72 — an `AtomicBool::swap` in early boot hangs the board
//! silently, and emulators do not reproduce it. Rather than ask every future
//! author to remember that, the window is removed.
//!
//! **Kernel map** ([`activate`]): the real per-region map with W^X and a guard
//! page, built at runtime from the linker layout and installed by switching
//! `TTBR0_EL1`. Because the early map is already active, the tables are written
//! through the caches the walker itself reads, so this needs a barrier rather
//! than the invalidate-the-world dance a cold enable requires.
//!
//! Which physical ranges are RAM and which are device MMIO is board knowledge,
//! so [`activate`]'s caller supplies them. The bit encodings and the region
//! splitting live in [`kernel_core::paging`] and are unit-tested on the host.

use kernel_core::paging::{self, Level, MemKind, Perms};

use crate::arch::aarch64::cache;
use crate::sync::SyncCell;

/// `TCR_EL1.T0SZ` — 39-bit VA, so the initial lookup level is 1.
const T0SZ: u64 = 25;

/// Writable and executable — only ever used by the early map, which cannot
/// distinguish `.text` from the stack at 1 GiB granularity. [`activate`]
/// replaces it before any of the kernel's own untrusted input is touched.
const RWX: Perms = Perms {
    write: true,
    execute: true,
};

/// Encode a level-1 block or fail the build.
///
/// The early map has no error path: it is installed before the console exists,
/// so a bad descriptor could not be reported. Evaluating it in `const` context
/// turns that into a compile error instead.
const fn early_block(pa: u64, kind: MemKind, perms: Perms) -> u64 {
    match paging::leaf(Level::L1, pa, kind, perms) {
        Some(descriptor) => descriptor,
        None => panic!("early identity map: unencodable block address"),
    }
}

/// The early identity map, resolved at compile time.
///
/// Not `mut` and never written at runtime: the table lives in `.rodata`, and
/// the page-table walker only reads it (the access flag is pre-set, so there
/// is no hardware update either).
static EARLY_L1: Table = Table {
    entries: {
        let mut entries = [0u64; paging::ENTRIES_PER_TABLE];
        // RAM: 0–3 GiB. Executable because the kernel image is at 0x80000, and
        // covering 3 GiB means a firmware-placed DTB is readable wherever it
        // landed — the fine map deliberately covers far less.
        entries[0] = early_block(0x0000_0000, MemKind::NormalWb, RWX);
        entries[1] = early_block(0x4000_0000, MemKind::NormalWb, RWX);
        entries[2] = early_block(0x8000_0000, MemKind::NormalWb, RWX);
        // Peripherals and the GIC.
        entries[3] = early_block(0xC000_0000, MemKind::Device, Perms::RW);
        entries
    },
};

/// Enable translation with the compile-time identity map.
///
/// Called from `_start` once the stack exists and BSS is clear, before
/// `kernel_main`. After this returns, memory has attributes and the rest of
/// the kernel — atomics included — behaves as the architecture documents.
///
/// # Safety
/// Call exactly once, on the primary core, with interrupts masked and
/// translation off.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn early_mmu_enable() {
    unsafe {
        // Caches are about to be enabled. Anything the firmware left resident
        // would otherwise shadow memory — including this table.
        cache::invalidate_dcache_all();
        cache::invalidate_icache();

        program_regime(&raw const EARLY_L1 as u64);
        cache::invalidate_tlb_all();
        enable_translation();
    }
}

unsafe extern "C" {
    static __pagetables_start: u8;
    static __pagetables_end: u8;
}

/// One translation table: 512 entries, page aligned at every level.
#[repr(C, align(4096))]
struct Table {
    entries: [u64; paging::ENTRIES_PER_TABLE],
}

/// Hands out zeroed translation tables from the linker-provided arena.
struct Arena {
    next: usize,
    end: usize,
}

/// Table arena state. Touched only while building the map, MMU off, IRQs masked.
static ARENA: SyncCell<Arena> = SyncCell::new(Arena { next: 0, end: 0 });

/// Physical address of the root table, published for `TTBR0_EL1`.
static ROOT: SyncCell<usize> = SyncCell::new(0);

/// Why a mapping request could not be satisfied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MmuError {
    /// `va`, `pa` or `len` was not page aligned.
    Unaligned { va: u64, pa: u64, len: u64 },
    /// A descriptor could not be encoded for this address at this level.
    BadDescriptor { va: u64, pa: u64 },
    /// The address is outside the 512 GiB the level-1 table describes.
    OutOfRange(u64),
    /// The table arena is exhausted — raise `PAGE_TABLE_ARENA_SIZE` in `link.ld`.
    OutOfTables,
    /// A region overlaps one already mapped with a block at a coarser level.
    ///
    /// Splitting an existing block is possible but never needed here: regions
    /// are mapped once, in address order, at boot.
    BlockAlreadyMapped(u64),
}

/// A region to identity-map.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub base: u64,
    pub len: u64,
    pub kind: MemKind,
    pub perms: Perms,
    /// Shown in diagnostics when mapping fails.
    pub name: &'static str,
}

/// Build the kernel map from `regions` and switch `TTBR0_EL1` to it.
///
/// Replaces the coarse early map. Anything not covered by a region becomes
/// unmapped and faults — that is the point of the guard page, and the reason
/// this is a distinct step rather than something `early_mmu_enable` could do.
///
/// On error nothing is switched: the early map stays active, so the caller can
/// report the failure over a console that still works.
///
/// # Safety
/// Single core, IRQs masked, early map active. Every address the kernel touches
/// after this returns must be inside one of `regions`.
pub unsafe fn activate(regions: &[Region]) -> Result<(), (MmuError, &'static str)> {
    unsafe {
        arena_init();

        let root = alloc_table().map_err(|e| (e, "root table"))?;

        for region in regions {
            map_region(root, region).map_err(|e| (e, region.name))?;
        }

        *ROOT.get() = root as usize;
        switch_ttbr0(root as u64);
    }
    Ok(())
}

/// Point `TTBR0_EL1` at a new table and flush the old translations.
///
/// No cache maintenance: translation is already on, so the table writes and
/// the walker's reads go through the same caches. Only ordering is needed —
/// the writes must be observable before the switch.
///
/// # Safety
/// `root` is a complete table covering every address in use.
unsafe fn switch_ttbr0(root: u64) {
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "msr ttbr0_el1, {root}",
            "isb",
            "tlbi vmalle1",
            "dsb ish",
            "isb",
            root = in(reg) root,
            options(nostack),
        );
    }
}

/// Point the arena at the linker-reserved range.
///
/// # Safety
/// Call once, before any [`alloc_table`].
unsafe fn arena_init() {
    unsafe {
        *ARENA.get() = Arena {
            next: core::ptr::addr_of!(__pagetables_start) as usize,
            end: core::ptr::addr_of!(__pagetables_end) as usize,
        };
    }
}

/// Take the next zeroed table from the arena.
///
/// # Safety
/// Single core, arena initialised.
unsafe fn alloc_table() -> Result<*mut Table, MmuError> {
    unsafe {
        let arena = &mut *ARENA.get();
        let size = core::mem::size_of::<Table>();
        if arena.next + size > arena.end {
            return Err(MmuError::OutOfTables);
        }

        let table = arena.next as *mut Table;
        arena.next += size;

        // The arena is NOLOAD and `boot.s` only clears .bss, so a table arrives
        // holding whatever was in DRAM. An unzeroed table is a walk into noise.
        (*table).entries = [0; paging::ENTRIES_PER_TABLE];

        Ok(table)
    }
}

/// Identity-map one region.
///
/// # Safety
/// `root` is a live level-1 table; MMU off.
unsafe fn map_region(root: *mut Table, region: &Region) -> Result<(), MmuError> {
    let chunks =
        paging::chunks(region.base, region.base, region.len).ok_or(MmuError::Unaligned {
            va: region.base,
            pa: region.base,
            len: region.len,
        })?;

    for chunk in chunks {
        unsafe { map_chunk(root, chunk, region.kind, region.perms)? };
    }
    Ok(())
}

/// Install one leaf descriptor, creating intermediate tables as needed.
///
/// # Safety
/// `root` is a live level-1 table; MMU off.
unsafe fn map_chunk(
    root: *mut Table,
    chunk: paging::Chunk,
    kind: MemKind,
    perms: Perms,
) -> Result<(), MmuError> {
    unsafe {
        // Level 1 covers 512 GiB; beyond that there is no entry to write.
        if chunk.va >= (paging::L1_BLOCK_SIZE * paging::ENTRIES_PER_TABLE as u64) {
            return Err(MmuError::OutOfRange(chunk.va));
        }

        let mut table = root;
        let mut level = Level::L1;

        // Walk down to the level this chunk maps, creating tables on the way.
        while level != chunk.level {
            let index = level.index(chunk.va);
            let entry = (*table).entries[index];

            let next = if entry == 0 {
                let new = alloc_table()?;
                (*table).entries[index] =
                    paging::table_descriptor(new as u64).ok_or(MmuError::BadDescriptor {
                        va: chunk.va,
                        pa: new as u64,
                    })?;
                new
            } else if entry & 0b11 == paging::DESC_TABLE {
                (entry & 0x0000_FFFF_FFFF_F000) as *mut Table
            } else {
                // A coarser block already covers this address.
                return Err(MmuError::BlockAlreadyMapped(chunk.va));
            };

            table = next;
            level = level.next().ok_or(MmuError::OutOfRange(chunk.va))?;
        }

        let index = level.index(chunk.va);
        (*table).entries[index] =
            paging::leaf(level, chunk.pa, kind, perms).ok_or(MmuError::BadDescriptor {
                va: chunk.va,
                pa: chunk.pa,
            })?;

        Ok(())
    }
}

/// Program the translation regime: `MAIR`, `TCR` and the initial `TTBR0`.
///
/// # Safety
/// `root` is a complete translation table covering every address in use.
unsafe fn program_regime(root: u64) {
    unsafe {
        core::arch::asm!(
            "msr mair_el1, {mair}",
            "msr tcr_el1, {tcr}",
            "msr ttbr0_el1, {root}",
            "isb",
            mair = in(reg) paging::mair_el1(),
            tcr = in(reg) paging::tcr_el1_ttbr0_only(T0SZ),
            root = in(reg) root,
            options(nostack),
        );
    }
}

/// Set `SCTLR_EL1.{M,C,I}` — translation and both caches.
///
/// # Safety
/// A valid regime must already be programmed; the code executing this call
/// must be identity-mapped, or the next instruction fetch faults.
unsafe fn enable_translation() {
    unsafe {
        let mut sctlr: u64;
        core::arch::asm!("mrs {v}, sctlr_el1", v = out(reg) sctlr, options(nostack));
        sctlr |= (1 << 0) | (1 << 2) | (1 << 12); // M | C | I
        sctlr &= !(1 << 1); // clear strict alignment bit
        core::arch::asm!(
            "msr sctlr_el1, {v}",
            "isb",
            v = in(reg) sctlr,
            options(nostack),
        );
    }
}

/// Bytes of the table arena still unused. Zero means the next map fails.
pub fn tables_remaining() -> usize {
    // SAFETY: single core; the arena is only mutated while building the map.
    let arena = unsafe { &*ARENA.get() };
    arena.end.saturating_sub(arena.next)
}

/// True if `SCTLR_EL1.M` is set.
pub fn is_enabled() -> bool {
    let sctlr: u64;
    // SAFETY: system register read.
    unsafe {
        core::arch::asm!("mrs {v}, sctlr_el1", v = out(reg) sctlr, options(nostack, nomem));
    }
    sctlr & 1 != 0
}
