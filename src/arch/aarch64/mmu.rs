//! AArch64 stage-1 MMU: multi-level identity map (4 KiB granule, 39-bit VA).
//!
//! Regions are mapped one at a time with their own permissions, using 1 GiB
//! and 2 MiB blocks where the alignment allows and 4 KiB pages where it does
//! not. That is what makes W^X and a stack guard page possible: with a single
//! 1 GiB block per gigabyte, `.text`, the stack and the heap all had to share
//! one set of permissions, and the weakest won.
//!
//! Which physical ranges are RAM and which are device MMIO is board knowledge,
//! so the caller supplies them. The bit encodings and the region splitting
//! live in [`kernel_core::paging`] and are unit-tested on the host.

use kernel_core::paging::{self, Level, MemKind, Perms};

use crate::arch::aarch64::cache;
use crate::sync::SyncCell;

/// `TCR_EL1.T0SZ` — 39-bit VA, so the initial lookup level is 1.
const T0SZ: u64 = 25;

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

/// Build the identity map from `regions`, then enable the MMU and caches.
///
/// Anything not covered by a region stays unmapped and faults — that is the
/// point of the guard page.
///
/// # Safety
/// Single core, MMU off, IRQs masked. Every address the kernel touches after
/// this returns must be inside one of `regions`.
pub unsafe fn enable(regions: &[Region]) -> Result<(), (MmuError, &'static str)> {
    unsafe {
        arena_init();

        let root = alloc_table().map_err(|e| (e, "root table"))?;
        *ROOT.get() = root as usize;

        for region in regions {
            map_region(root, region).map_err(|e| (e, region.name))?;
        }

        install(root as u64);
    }
    Ok(())
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

/// Program `MAIR`/`TCR`/`TTBR0`, invalidate, then set `SCTLR_EL1.{M,C,I}`.
///
/// # Safety
/// `root` is a complete translation table covering every address in use.
unsafe fn install(root: u64) {
    unsafe {
        core::arch::asm!(
            "msr mair_el1, {v}",
            v = in(reg) paging::mair_el1(),
            options(nostack),
        );
        core::arch::asm!(
            "msr tcr_el1, {v}",
            v = in(reg) paging::tcr_el1_ttbr0_only(T0SZ),
            options(nostack),
        );
        core::arch::asm!(
            "msr ttbr0_el1, {root}",
            "isb",
            root = in(reg) root,
            options(nostack),
        );

        // The tables were written with the MMU off, so they went straight to
        // memory — but the walker is about to read them through the caches, and
        // the firmware left lines of its own behind.
        cache::invalidate_dcache_all();
        cache::invalidate_icache();
        cache::invalidate_tlb_all();

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
