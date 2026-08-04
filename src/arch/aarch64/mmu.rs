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

use kernel_core::layout::Region;
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

/// Table arena state.
///
/// Mutated by `activate` while the early map is active, and by `map` with the
/// kernel map active — never with translation off, and always with interrupts
/// masked. The invariant is the masking, not the MMU state: the walker reads
/// the tables this hands out, so a half-written entry must not be reachable.
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
    /// [`map`] was called before [`activate`] installed the kernel map.
    NotActivated,
    /// Something is already mapped at this address, at this level.
    ///
    /// Distinct from [`Self::BlockAlreadyMapped`], which is a *coarser* block
    /// in the way: this is an exact collision, and overwriting it would change
    /// the target and permissions of a live mapping without saying so.
    AlreadyMapped(u64),
    /// [`unmap`] found no live mapping covering this address.
    NotMapped(u64),
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

/// Add `region` to the live kernel map.
///
/// The map is built once by [`activate`], before any address the firmware
/// assigns at runtime is known — a device-tree blob, a framebuffer, later a
/// task's memory. This is how such a region joins it afterwards.
///
/// # Safety
/// [`activate`] must have succeeded, and `region` must not conflict with an
/// existing mapping. Called with interrupts masked: it mutates live tables and
/// the arena, neither of which is protected against a concurrent walker.
pub unsafe fn map(region: &Region) -> Result<(), MmuError> {
    unsafe {
        let root = *ROOT.get();
        if root == 0 {
            return Err(MmuError::NotActivated);
        }

        map_region(root as *mut Table, region)?;

        // Publish the entries before anything can walk them, then drop stale
        // translations. Going from invalid to valid would not strictly need the
        // invalidation — the architecture does not permit caching invalid
        // entries — but the same path is shared with [`unmap`], where it is
        // mandatory.
        publish_and_invalidate(region.base, region.len);
        Ok(())
    }
}

/// Remove the live mapping of `[base, base + len)`.
///
/// Every page in the range must already be mapped; on success each becomes a
/// translation fault. Used for task-stack guard pages (ADR-0006): the physical
/// page stays owned by the stack allocation — only the virtual mapping goes.
///
/// Coarser blocks that cover a page being unmapped are split into the next
/// level (break-before-make) so a single 4 KiB guard can sit inside a 2 MiB
/// heap block without unmapping the rest of the heap.
///
/// # Safety
/// [`activate`] must have succeeded. Interrupts masked. After this returns,
/// software must not touch the range except through a deliberate remapping;
/// instruction fetches or data accesses there will fault.
pub unsafe fn unmap(base: u64, len: u64) -> Result<(), MmuError> {
    unsafe {
        let root = *ROOT.get();
        if root == 0 {
            return Err(MmuError::NotActivated);
        }
        if base % paging::PAGE_SIZE != 0 || len % paging::PAGE_SIZE != 0 || len == 0 {
            return Err(MmuError::Unaligned {
                va: base,
                pa: base,
                len,
            });
        }

        let root = root as *mut Table;
        let end = base.checked_add(len).ok_or(MmuError::OutOfRange(base))?;

        // Fail closed before mutating: a hole mid-range would leave a half
        // unmapped region with no honest recovery short of a reboot.
        let mut va = base;
        while va < end {
            ensure_mapped(root, va)?;
            va += paging::PAGE_SIZE;
        }

        va = base;
        while va < end {
            unmap_page(root, va)?;
            va += paging::PAGE_SIZE;
        }

        publish_and_invalidate(base, len);
        Ok(())
    }
}

/// `dsb ishst`, TLB invalidation for `[va, va + len)`, then `dsb ish` / `isb`.
///
/// # Safety
/// Table updates for the range must already be visible to this core's view of
/// memory; this only orders and invalidates.
unsafe fn publish_and_invalidate(va: u64, len: u64) {
    unsafe {
        core::arch::asm!("dsb ishst", options(nostack, preserves_flags));

        match paging::tlbi_plan(va, len) {
            Some(paging::TlbiPlan::ByPage { first, pages }) => {
                for index in 0..pages {
                    let operand = paging::tlbi_operand(first, index);
                    core::arch::asm!(
                        "tlbi vaae1is, {op}",
                        op = in(reg) operand,
                        options(nostack, preserves_flags),
                    );
                }
            }
            // Large region, or a planner refusal: the whole TLB is always enough.
            _ => core::arch::asm!("tlbi vmalle1", options(nostack, preserves_flags)),
        }

        core::arch::asm!("dsb ish", "isb", options(nostack, preserves_flags));
    }
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
/// `root` is a live level-1 table and interrupts are masked. Translation may be
/// on: this runs under `activate` (early map active) and under `map` (kernel
/// map active), so it must not assume either.
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
/// `root` is a live level-1 table and interrupts are masked. Translation may be
/// on — see [`map_region`]. The caller publishes the entries and invalidates;
/// this function only writes them.
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

        // Refuse to overwrite. Silently replacing a leaf changes the physical
        // address and the permissions of something already in use — a W^X
        // region quietly becoming writable is the failure this map exists to
        // prevent. Changing an existing mapping is a legitimate operation, but
        // it has to be asked for: a future `remap` can clear the entry first,
        // which also makes the break-before-make sequence its author's problem
        // rather than an accident.
        if (*table).entries[index] != 0 {
            return Err(MmuError::AlreadyMapped(chunk.va));
        }

        (*table).entries[index] =
            paging::leaf(level, chunk.pa, kind, perms).ok_or(MmuError::BadDescriptor {
                va: chunk.va,
                pa: chunk.pa,
            })?;

        Ok(())
    }
}

/// Walk without mutating: `va` must be covered by some leaf (block or page).
///
/// # Safety
/// `root` is the live level-1 table; interrupts masked.
unsafe fn ensure_mapped(root: *mut Table, va: u64) -> Result<(), MmuError> {
    unsafe {
        if va >= (paging::L1_BLOCK_SIZE * paging::ENTRIES_PER_TABLE as u64) {
            return Err(MmuError::OutOfRange(va));
        }

        let mut table = root;
        let mut level = Level::L1;

        loop {
            let index = level.index(va);
            let entry = (*table).entries[index];

            if paging::is_invalid(entry) {
                return Err(MmuError::NotMapped(va));
            }
            if paging::is_leaf(entry, level) {
                return Ok(());
            }
            if paging::is_table(entry, level) {
                table = paging::descriptor_address(entry) as *mut Table;
                level = level.next().ok_or(MmuError::OutOfRange(va))?;
                continue;
            }
            return Err(MmuError::NotMapped(va));
        }
    }
}

/// Clear the L3 mapping for one page, splitting coarser blocks as needed.
///
/// # Safety
/// `root` is the live level-1 table; interrupts masked; [`ensure_mapped`] has
/// succeeded for `va`. Intermediate break-before-make flushes run inside.
unsafe fn unmap_page(root: *mut Table, va: u64) -> Result<(), MmuError> {
    unsafe {
        let mut table = root;
        let mut level = Level::L1;

        loop {
            let index = level.index(va);
            let entry = (*table).entries[index];

            if paging::is_invalid(entry) {
                return Err(MmuError::NotMapped(va));
            }

            if paging::is_leaf(entry, level) {
                if level == Level::L3 {
                    (*table).entries[index] = 0;
                    return Ok(());
                }
                // Coarser block covers this page: replace it with a table of
                // next-level leaves, then continue the walk into that table.
                table = split_block(table, index, level, entry, va)?;
                level = level.next().ok_or(MmuError::OutOfRange(va))?;
                continue;
            }

            if paging::is_table(entry, level) {
                table = paging::descriptor_address(entry) as *mut Table;
                level = level.next().ok_or(MmuError::OutOfRange(va))?;
                continue;
            }

            return Err(MmuError::NotMapped(va));
        }
    }
}

/// Break-before-make: replace a block leaf with a fully populated next-level
/// table that maps the same physical range with the same attributes.
///
/// Returns a pointer to the new table. The caller's `va` must lie inside the
/// block being split (used only to compute the block's base for TLB flush).
///
/// # Safety
/// Live tables; IRQs masked. Allocates from the page-table arena.
unsafe fn split_block(
    parent: *mut Table,
    index: usize,
    level: Level,
    block_entry: u64,
    va: u64,
) -> Result<*mut Table, MmuError> {
    unsafe {
        let (pa_base, kind, perms) =
            paging::decode_leaf(block_entry, level).ok_or(MmuError::BadDescriptor {
                va,
                pa: paging::descriptor_address(block_entry),
            })?;
        let next_level = level.next().ok_or(MmuError::OutOfRange(va))?;
        let child = alloc_table()?;
        let entry_size = next_level.entry_size();

        for i in 0..paging::ENTRIES_PER_TABLE {
            let pa = pa_base + (i as u64) * entry_size;
            (*child).entries[i] = paging::leaf(next_level, pa, kind, perms)
                .ok_or(MmuError::BadDescriptor { va, pa })?;
        }

        // Break: drop the block so no walker can see a half-built child.
        (*parent).entries[index] = 0;
        let block_size = level.entry_size();
        let block_va = va & !(block_size - 1);
        publish_and_invalidate(block_va, block_size);

        // Make: install the table. Invalid→valid; still ordered for the next walk.
        (*parent).entries[index] =
            paging::table_descriptor(child as u64).ok_or(MmuError::BadDescriptor {
                va,
                pa: child as u64,
            })?;
        core::arch::asm!("dsb ishst", options(nostack, preserves_flags));

        Ok(child)
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
