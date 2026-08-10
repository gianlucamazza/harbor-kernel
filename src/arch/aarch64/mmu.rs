//! AArch64 stage-1 MMU: two maps, in the order Linux uses.
//!
//! **Early map** ([`enable_identity`]): the sequence that turns translation on
//! with a table someone else built. The table itself is
//! [`crate::mm::early`] — which gigabyte is RAM and which is device MMIO is
//! board knowledge, and this tree is reserved for CPU and ISA (F23). What is
//! architectural is the *order*: caches invalidated while they are still off,
//! then the regime programmed, then the TLB dropped, and only then translation
//! enabled.
//!
//! **Kernel map** ([`activate`]): the real per-region map with W^X and a guard
//! page, built at runtime from the linker layout and installed by switching
//! `TTBR0_EL1`. Because the early map is already active, the tables are written
//! through the caches the walker itself reads, so this needs a barrier rather
//! than the invalidate-the-world dance a cold enable requires.
//!
//! Which physical ranges are RAM and which are device MMIO is board knowledge,
//! so both maps take it from the BSP: [`activate`]'s caller supplies the
//! regions, and [`crate::mm::early`] supplies the early table. The bit
//! encodings and the region splitting live in [`kernel_core::paging`] and are
//! unit-tested on the host.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use kernel_core::layout::Region;
use kernel_core::paging::{self, Level, MemKind, Perms};

use crate::arch::cache;
use crate::sync::SyncCell;

/// Blocks split into next-level tables since boot. See [`splits`].
static SPLITS: AtomicU32 = AtomicU32::new(0);

/// `TCR_EL1.T0SZ` — 39-bit VA, so the initial lookup level is 1.
const T0SZ: u64 = 25;

/// Enable translation with a caller-supplied identity map.
///
/// Called from `crate::mm::early::early_mmu_enable`, which `boot.s` branches to
/// once the stack exists and BSS is clear. After this returns, memory has
/// attributes and the rest of the kernel — atomics included — behaves as the
/// architecture documents.
///
/// # Safety
/// `root` must be a complete translation table covering every address in use,
/// and live for the rest of the kernel's life. Call exactly once, on the
/// primary core, with interrupts masked and translation off.
pub unsafe fn enable_identity(root: u64) {
    // SAFETY: order is load-bearing and cannot be rearranged: caches invalidated while
    // they are still off, then the regime programmed, then the TLB dropped, and
    // only then translation enabled. Each step assumes the previous one.
    unsafe {
        // Caches are about to be enabled. Anything the firmware left resident
        // would otherwise shadow memory — including the caller's table.
        cache::invalidate_dcache_all();
        cache::invalidate_icache();

        program_regime(root);
        cache::invalidate_tlb_all();
        enable_translation();
    }
}

unsafe extern "C" {
    static __pagetables_start: u8;
    static __pagetables_end: u8;
}

/// One translation table: 512 entries, page aligned at every level.
///
/// Public because the early map is built outside this module and the *format*
/// is architectural even when the contents are not.
#[repr(C, align(4096))]
pub struct Table {
    pub entries: [u64; paging::ENTRIES_PER_TABLE],
}

/// Hands out zeroed translation tables from the linker-provided arena.
struct Arena {
    next: usize,
    end: usize,
}

/// Table arena state.
///
/// Mutated by `activate` while the early map is active, and by `map`/`unmap`
/// under [`MAP_LOCK`] (ADR-0077 / F-R1-P1): dual-current cores may unmap stack
/// guards concurrently. The walker reads published entries after
/// `publish_and_invalidate`.
static ARENA: SyncCell<Arena> = SyncCell::new(Arena { next: 0, end: 0 });

/// Serialises kernel map mutation and arena bump (not taken from IRQ handlers).
static MAP_LOCK: crate::sync::IrqSpinLock = crate::sync::IrqSpinLock::new();

/// Physical address of the root table, published for `TTBR0_EL1`.
///
/// An atomic rather than a `SyncCell`, because the access pattern is not the
/// one `SyncCell` describes: this is written exactly once, by `activate`, and
/// only read afterwards. As a `SyncCell` every reader needed `unsafe` and
/// `kernel_root_phys` was reading it from a safe `pub fn` with no mask — the
/// only accessor in the tree that did. Publication with `Release` / `Acquire`
/// says write-once-then-read in the type, and costs nothing on this core.
static ROOT: AtomicUsize = AtomicUsize::new(0);

/// Kernel page-table root physical address (0 if not activated).
///
/// Used by M5 user AS prepare (ADR-0014) to deep-clone kernel coverage.
#[inline]
pub fn kernel_root_phys() -> Option<usize> {
    match ROOT.load(Ordering::Acquire) {
        0 => None,
        root => Some(root),
    }
}

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
    // SAFETY: the sequence is the contract: arena first, then the root, then the regions,
    // and `ROOT` published only once every region mapped. On any error nothing is
    // switched and the early map stays live, which is what lets the failure be
    // reported over a working console.
    unsafe {
        arena_init();

        let root = alloc_table().map_err(|e| (e, "root table"))?;

        for region in regions {
            map_region(root, region).map_err(|e| (e, region.name))?;
        }

        ROOT.store(root as usize, Ordering::Release);
        switch_ttbr0(root as u64);
        retire_early_map();
    }
    Ok(())
}

/// Drop every TLB entry the early map left behind. Called once, by
/// [`activate`], immediately after the switch to the fine-grained root.
///
/// The early map is 1 GiB **Global** L1 blocks, RWX at EL1 and EL0-denied
/// (`mm::early`). Global entries match every ASID and survive an ASID-scoped
/// regime: since ADR-0050 removed the per-switch `tlbi vmalle1is`, nothing
/// retired them — and on Cortex-A72 (which fills the TLB speculatively, unlike
/// QEMU) a stale early block served the first EL0 fetches at `USER_VA_BASE` as
/// *its* 1 GiB translation: instruction abort, permission fault level 1, seen
/// on Pi 4B 2026-08-09 (`.serial-log/20260809-093312.log`). Until evicted they
/// also shadow the fine map's W^X attributes for EL1. Ending the early map's
/// life is this boundary's job, so the one full invalidate lives here — the
/// per-switch path stays TLBI-free (ADR-0050 §3, amended).
unsafe fn retire_early_map() {
    // SAFETY: invalidate-only; the fine root is installed and covers every
    // address in use, so re-walking after the wipe resolves through it.
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "tlbi vmalle1is",
            "dsb ish",
            "isb",
            options(nostack, preserves_flags),
        );
    }
}

/// Add `region` to the live kernel map.
///
/// The map is built once by [`activate`], before any address the firmware
/// assigns at runtime is known — a device-tree blob, a framebuffer, later a
/// task's memory. This is how such a region joins it afterwards.
///
/// # Safety
/// [`activate`] must have succeeded, and `region` must not conflict with an
/// existing mapping. Serialised by [`MAP_LOCK`] (callers need not mask IRQs
/// themselves, though many still do).
pub unsafe fn map(region: &Region) -> Result<(), MmuError> {
    MAP_LOCK.with(|| {
        // SAFETY: root is non-zero (checked) and points at the live L1 table
        // published by `activate`; MAP_LOCK excludes concurrent map/unmap.
        unsafe {
            let root = ROOT.load(Ordering::Acquire);
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
    })
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
/// [`activate`] must have succeeded. After this returns, software must not
/// touch the range except through a deliberate remapping; instruction fetches
/// or data accesses there will fault. Serialised by [`MAP_LOCK`].
pub unsafe fn unmap(base: u64, len: u64) -> Result<(), MmuError> {
    MAP_LOCK.with(|| {
    // SAFETY: root is non-zero (checked); MAP_LOCK excludes concurrent map/unmap.
    // `ensure_mapped` runs before any page is cleared, so a range that is only
    // partly mapped is refused whole rather than half-unmapped.
    unsafe {
        let root = ROOT.load(Ordering::Acquire);
        if root == 0 {
            return Err(MmuError::NotActivated);
        }
        if !base.is_multiple_of(paging::PAGE_SIZE)
            || !len.is_multiple_of(paging::PAGE_SIZE)
            || len == 0
        {
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
    })
}

/// `dsb ishst`, TLB invalidation for `[va, va + len)`, then `dsb ish` / `isb`.
///
/// # Safety
/// Table updates for the range must already be visible to this core's view of
/// memory; this only orders and invalidates.
unsafe fn publish_and_invalidate(va: u64, len: u64) {
    // SAFETY: only orders and invalidates — writes nothing. The caller has already made
    // the table updates; this is what makes them visible to the walker and drops
    // the stale entries.
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
            // `is`, matching the per-page branch above and the `dsb ish` that
            // closes the sequence — the operation and its barrier must name the
            // same shareability domain or the barrier is ordering a scope the
            // operation never reached.
            _ => core::arch::asm!("tlbi vmalle1is", options(nostack, preserves_flags)),
        }

        core::arch::asm!("dsb ish", "isb", options(nostack, preserves_flags));
    }
}

/// Point `TTBR0_EL1` at a new table and publish its ASID.
///
/// **Sole** TTBR0 switch (boot [`activate`], EL0 entry, lower-EL restore).
/// Rust and asm (`vectors.s`, `el0_run`) all call this symbol — no parallel
/// barrier sequences.
///
/// `ttbr` is the full register value: physical root in the low bits and ASID
/// in [63:48] (ADR-0050). Kernel switches use ASID 0 (plain phys root). User
/// switches pass [`crate::mm::AddressSpace::ttbr0_value`].
///
/// No global TLBI on switch: user leaves are non-global (`nG`) and ASID-tagged,
/// kernel leaves stay Global. Stale tags are flushed with [`invalidate_asid`]
/// when an AS is destroyed. Barriers alone order the TTBR/CONTEXTIDR writes.
///
/// # Safety
/// `ttbr`'s root is a complete table covering every address in use under the
/// new root. For EL0 entry the root must include kernel coverage plus the user
/// window (ADR-0014). IRQs should be masked across a switch paired with EL change.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn switch_ttbr0(ttbr: u64) {
    // Null root is never a valid identity map; installing it would make the
    // next fetch unrecoverable. Callers (vectors, el0_run) must pass a real root.
    // ASID bits live above bit 48 — strip them before the null check.
    let root = ttbr & ((1u64 << 48) - 1);
    if root == 0 {
        panic!("switch_ttbr0: refused null root");
    }
    let asid = (ttbr >> 48) & 0xFFFF;
    // SAFETY: barriers around the `TTBR0` / `CONTEXTIDR` writes: `dsb ishst`
    // so table writes are visible to the walker first, `isb` so the switch
    // takes effect before the next fetch. No `tlbi vmalle1is` — see doc.
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "msr ttbr0_el1, {ttbr}",
            "msr contextidr_el1, {asid}",
            "isb",
            "dsb ish",
            "isb",
            ttbr = in(reg) ttbr,
            asid = in(reg) asid,
            options(nostack),
        );
    }
}

/// Invalidate all TLB entries tagged with `asid` (inner-shareable).
///
/// Call before reusing an ASID after the owning address space is destroyed.
/// ASID 0 is a no-op (kernel global leaves are not ASID-tagged).
pub fn invalidate_asid(asid: u16) {
    if asid == 0 {
        return;
    }
    // TLBI ASIDE1IS operand: ASID in bits [15:0] of Xt (AArch64).
    let op = asid as u64;
    // SAFETY: ASID-scoped invalidate; does not remove Global entries.
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "tlbi aside1is, {op}",
            "dsb ish",
            "isb",
            op = in(reg) op,
            options(nostack),
        );
    }
}

/// Point the arena at the linker-reserved range.
///
/// # Safety
/// Call once, before any [`alloc_table`].
unsafe fn arena_init() {
    // SAFETY: sole writer, before any `alloc_table` — the function's contract. The linker
    // symbols bound a region reserved by `link.ld` and never otherwise written.
    unsafe {
        *ARENA.get() = Arena {
            next: &raw const __pagetables_start as usize,
            end: &raw const __pagetables_end as usize,
        };
    }
}

/// Take the next zeroed table from the arena.
///
/// # Safety
/// Caller holds [`MAP_LOCK`] (or is the single-threaded `activate` path before
/// secondary cores run).
unsafe fn alloc_table() -> Result<*mut Table, MmuError> {
    // SAFETY: exclusive `&mut` via MAP_LOCK / activate-only boot.
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
        // SAFETY: each chunk came from the planner, which only emits addresses inside the
        // region the caller asked for — so this cannot map memory the caller did not
        // request.
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
    // SAFETY: writes one descriptor into a table reached from `root`. The level bound is
    // checked first, so no write lands outside the 512 GiB the L1 table covers.
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
    // SAFETY: read-only walk. Every dereference is of a table this walk reached through a
    // valid table descriptor, starting from the live root the caller supplied.
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
    // SAFETY: `ensure_mapped` has already proved a leaf covers `va`, so every descriptor
    // this walk reads exists. Splits it performs are individually published, per
    // the function's contract.
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
    // SAFETY: break-before-make: the block is cleared and invalidated before the table
    // replacing it is installed, so no walker can observe both mappings. The
    // decode-then-re-encode round trip is what keeps the split's leaves
    // equivalent to the block they came from.
    unsafe {
        let (pa_base, kind, perms) =
            paging::decode_leaf(block_entry, level).ok_or(MmuError::BadDescriptor {
                va,
                pa: paging::descriptor_address(block_entry),
            })?;
        let next_level = level.next().ok_or(MmuError::OutOfRange(va))?;
        let child = alloc_table()?;
        // Counted because the arena is a bump with no free path: a split takes
        // a table for the rest of the boot, and `release` remaps the leaf
        // without giving the table back. A resource consumed by a path nobody
        // counts is an exhaustion nobody sees coming — the same argument as
        // `mm::refused_frees`.
        SPLITS.fetch_add(1, Ordering::Relaxed);
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
    // SAFETY: writes MAIR/TCR/TTBR0 and ends with `isb`, so the regime is in force before
    // the caller enables translation. Ordering, not just the values, is what the
    // `# Safety` above depends on.
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
    // SAFETY: the read-modify-write is the point: `boot.s` has already programmed the
    // RES1 pattern into this register, and overwriting rather than or-ing would
    // clear it again.
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

/// Ask the MMU whether `va` translates, using the hardware walker.
///
/// `AT S1E1R` runs the same stage-1 walk a load would, and `PAR_EL1.F` says
/// whether it faulted — so this answers about the table that is *installed*,
/// not about the table someone believes was installed. That distinction is the
/// whole point here: task-stack guard pages are unmapped after the map is
/// built (ADR-0006), so a region table lookup would confidently place a stack
/// overflow "inside the heap", which is where the guard was carved from.
///
/// Read permission only: a write fault to a read-only page still translates,
/// and the syndrome already says it was a permission fault. Diagnostics only —
/// nothing maps or unmaps based on this.
pub fn translates(va: u64) -> bool {
    // SAFETY: `AT` performs a walk with no architectural side effects beyond
    // writing `PAR_EL1`, which nothing else in the kernel reads across this
    // sequence. `isb` orders the walk before the `PAR_EL1` read.
    let par: u64 = unsafe {
        let value: u64;
        core::arch::asm!(
            "at s1e1r, {va}",
            "isb",
            "mrs {out}, par_el1",
            va = in(reg) va,
            out = out(reg) value,
            options(nostack, preserves_flags),
        );
        value
    };
    // PAR_EL1.F: 0 = translation succeeded.
    par & 1 == 0
}

/// Bytes of the table arena still unused. Zero means the next map fails.
pub fn tables_remaining() -> usize {
    MAP_LOCK.with(|| {
        // SAFETY: exclusivity from MAP_LOCK vs alloc_table under map/unmap.
        let arena = unsafe { &*ARENA.get() };
        arena.end.saturating_sub(arena.next)
    })
}

/// Tables still available, in tables rather than bytes.
pub fn tables_free() -> usize {
    tables_remaining() / core::mem::size_of::<Table>()
}

/// Blocks split into next-level tables since boot.
///
/// Each split costs one table permanently: the arena is a bump allocator with
/// no free path, and unmapping a guard page inside a 2 MiB block splits that
/// block. Spawning tasks into distinct blocks therefore consumes the arena
/// monotonically, which is why boot refuses to continue below a reserve.
pub fn splits() -> u32 {
    SPLITS.load(Ordering::Relaxed)
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

/// Enable translation on a **secondary** core using the primary's live root.
///
/// Does not rebuild tables or republish [`kernel_root_phys`]. Used by the K8
/// first slice ([ADR-0070](../../../../docs/adr/0070-k8-smp-first-slice.md)).
///
/// # Safety
/// `root` must be the physical address already activated on core 0. Call once
/// per secondary, with IRQs masked, translation off, and the code path
/// identity-mapped under that root.
pub unsafe fn enable_existing(root: u64) {
    // SAFETY: same order as [`enable_identity`]: invalidate, program regime,
    // TLB drop, then enable. Tables are already complete on the primary.
    unsafe {
        cache::invalidate_dcache_all();
        cache::invalidate_icache();
        program_regime(root);
        cache::invalidate_tlb_all();
        enable_translation();
    }
}
