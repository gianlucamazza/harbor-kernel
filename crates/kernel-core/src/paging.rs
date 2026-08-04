//! AArch64 stage-1 translation encodings (4 KiB granule).
//!
//! Descriptor and `TCR_EL1` bit layouts, isolated from the `msr` sequence that
//! installs them. A wrong bit here is a silent fault or a walk through an
//! uninitialised table, neither of which is observable from a serial log.

/// Block descriptor at level 1 or 2.
pub const DESC_BLOCK: u64 = 0b01;
/// Table descriptor (levels 1 and 2) — and, confusingly, the *page* descriptor
/// at level 3, where `0b01` is invalid instead.
pub const DESC_TABLE: u64 = 0b11;
/// Page descriptor at level 3. Same encoding as [`DESC_TABLE`]; the level
/// decides how it is read.
pub const DESC_PAGE: u64 = 0b11;
/// Access flag: a cleared AF faults on first touch.
pub const DESC_AF: u64 = 1 << 10;
/// Inner shareable.
pub const DESC_SH_IS: u64 = 0b11 << 8;
/// Read/write at EL1, no EL0 access.
pub const DESC_AP_EL1_RW: u64 = 0b00 << 6;
/// Read-only at EL1, no EL0 access.
pub const DESC_AP_EL1_RO: u64 = 0b10 << 6;
/// Never execute at EL0.
pub const DESC_UXN: u64 = 1 << 54;
/// Never execute at EL1.
pub const DESC_PXN: u64 = 1 << 53;

/// `MAIR_EL1` attribute index 0: Normal, write-back, read/write allocate.
pub const ATTR_IDX_NORMAL: u64 = 0 << 2;
/// `MAIR_EL1` attribute index 1: Device-nGnRnE.
pub const ATTR_IDX_DEVICE: u64 = 1 << 2;

/// `MAIR_EL1` byte for Normal write-back.
pub const MAIR_NORMAL_WB: u64 = 0xFF;
/// `MAIR_EL1` byte for Device-nGnRnE.
pub const MAIR_DEVICE_NGNRNE: u64 = 0x00;

/// The assembled `MAIR_EL1` value matching the attribute indices above.
pub const fn mair_el1() -> u64 {
    MAIR_NORMAL_WB | (MAIR_DEVICE_NGNRNE << 8)
}

/// Size of a level-1 block with the 4 KiB granule.
pub const L1_BLOCK_SIZE: u64 = 1 << 30;
/// Size of a level-2 block.
pub const L2_BLOCK_SIZE: u64 = 1 << 21;
/// Size of a level-3 page — the granule.
pub const PAGE_SIZE: u64 = 1 << 12;

/// Entries per table at every level with the 4 KiB granule.
pub const ENTRIES_PER_TABLE: usize = 512;

/// Output-address field of a descriptor: bits [47:12].
const ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;

/// Translation level a descriptor lives at.
///
/// With `T0SZ = 25` the walk starts at level 1, so these are the three levels
/// the kernel ever writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// 1 GiB blocks.
    L1,
    /// 2 MiB blocks.
    L2,
    /// 4 KiB pages.
    L3,
}

impl Level {
    /// Bytes mapped by one entry at this level.
    pub const fn entry_size(self) -> u64 {
        match self {
            Level::L1 => L1_BLOCK_SIZE,
            Level::L2 => L2_BLOCK_SIZE,
            Level::L3 => PAGE_SIZE,
        }
    }

    /// Index into this level's table for `va`.
    pub const fn index(self, va: u64) -> usize {
        let shift = match self {
            Level::L1 => 30,
            Level::L2 => 21,
            Level::L3 => 12,
        };
        ((va >> shift) as usize) & (ENTRIES_PER_TABLE - 1)
    }

    /// The level below, or `None` at the last one.
    pub const fn next(self) -> Option<Level> {
        match self {
            Level::L1 => Some(Level::L2),
            Level::L2 => Some(Level::L3),
            Level::L3 => None,
        }
    }
}

/// Descriptor pointing at the next-level table at physical address `pa`.
///
/// Attributes are left to the leaf entries: the table-level permission fields
/// (APTable, XNTable…) only ever restrict further, and mixing the two makes
/// permissions unreadable at the point they matter.
pub const fn table_descriptor(pa: u64) -> Option<u64> {
    if pa % PAGE_SIZE != 0 || pa >= (1 << 48) {
        return None;
    }
    Some((pa & ADDR_MASK) | DESC_TABLE)
}

/// Encode a leaf descriptor at `level` mapping `pa`.
///
/// `None` if `pa` is not aligned to the level's entry size or does not fit the
/// 48-bit output address.
pub const fn leaf(level: Level, pa: u64, kind: MemKind, perms: Perms) -> Option<u64> {
    let size = level.entry_size();
    if pa % size != 0 || pa >= (1 << 48) {
        return None;
    }

    // Level 3 leaves are `0b11`; at levels 1 and 2 that encoding means "table",
    // and `0b01` means "block". Getting this backwards produces a descriptor
    // the walker either ignores or follows into nonsense.
    let kind_bits = match level {
        Level::L3 => DESC_PAGE,
        _ => DESC_BLOCK,
    };

    let mut desc = (pa & ADDR_MASK) | DESC_AF | DESC_SH_IS | kind_bits;

    desc |= match kind {
        MemKind::NormalWb => ATTR_IDX_NORMAL,
        MemKind::Device => ATTR_IDX_DEVICE,
    };

    desc |= if perms.write {
        DESC_AP_EL1_RW
    } else {
        DESC_AP_EL1_RO
    };

    // EL0 never executes a kernel mapping.
    desc |= DESC_UXN;
    if !perms.execute {
        desc |= DESC_PXN;
    }

    Some(desc)
}

/// True when the entry is free: the walker treats type `0b00` as invalid.
#[inline]
pub const fn is_invalid(entry: u64) -> bool {
    entry & 0b11 == 0
}

/// True when `entry` is a next-level table pointer at `level` (L1 or L2 only).
///
/// At L3 the same low bits mean "page", not "table" — see [`is_leaf`].
#[inline]
pub const fn is_table(entry: u64, level: Level) -> bool {
    matches!(level, Level::L1 | Level::L2) && (entry & 0b11) == DESC_TABLE
}

/// True when `entry` is a block (L1/L2) or page (L3) mapping.
#[inline]
pub const fn is_leaf(entry: u64, level: Level) -> bool {
    match level {
        Level::L3 => (entry & 0b11) == DESC_PAGE,
        Level::L1 | Level::L2 => (entry & 0b11) == DESC_BLOCK,
    }
}

/// Output address field of a table or leaf descriptor (bits [47:12]).
#[inline]
pub const fn descriptor_address(entry: u64) -> u64 {
    entry & ADDR_MASK
}

/// Decode a leaf written by [`leaf`] / [`l1_block`].
///
/// `None` if the entry is invalid, a table pointer, or not a recognisable leaf
/// at `level`. Used when splitting a coarser block so the child pages keep the
/// same memory type and permissions.
pub const fn decode_leaf(entry: u64, level: Level) -> Option<(u64, MemKind, Perms)> {
    if !is_leaf(entry, level) {
        return None;
    }

    let pa = descriptor_address(entry);
    // Attribute index lives in bits [4:2].
    let kind = if (entry & (0b111 << 2)) == ATTR_IDX_DEVICE {
        MemKind::Device
    } else {
        MemKind::NormalWb
    };

    // AP[2:1] at bits [7:6]: `00` EL1 RW, `10` EL1 RO (no EL0 in this kernel).
    let write = (entry & (0b11 << 6)) == DESC_AP_EL1_RW;
    let execute = (entry & DESC_PXN) == 0;

    Some((pa, kind, Perms { write, execute }))
}

/// One naturally aligned piece of a mapping request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub level: Level,
    pub va: u64,
    pub pa: u64,
}

impl Chunk {
    /// Bytes this chunk covers.
    pub const fn size(self) -> u64 {
        self.level.entry_size()
    }
}

/// Split `[va, va + len)` into the largest naturally aligned chunks.
///
/// A level is usable only where *both* addresses are aligned to it and the
/// remaining length still covers it, so a region whose VA and PA have different
/// alignments degrades to 4 KiB pages rather than silently mapping the wrong
/// physical memory.
#[derive(Clone, Copy, Debug)]
pub struct Chunks {
    va: u64,
    pa: u64,
    remaining: u64,
}

/// Iterate the chunks covering `[va, va + len)`.
///
/// `None` if `va`, `pa` or `len` is not page aligned — a partial page has no
/// correct rounding: growing it maps memory the caller did not ask for and
/// shrinking it leaves a hole.
pub const fn chunks(va: u64, pa: u64, len: u64) -> Option<Chunks> {
    if va % PAGE_SIZE != 0 || pa % PAGE_SIZE != 0 || len % PAGE_SIZE != 0 {
        return None;
    }
    Some(Chunks {
        va,
        pa,
        remaining: len,
    })
}

impl Iterator for Chunks {
    type Item = Chunk;

    fn next(&mut self) -> Option<Chunk> {
        if self.remaining == 0 {
            return None;
        }

        let level = [Level::L1, Level::L2, Level::L3]
            .into_iter()
            .find(|level| {
                let size = level.entry_size();
                self.va % size == 0 && self.pa % size == 0 && self.remaining >= size
            })
            // Every address is 4 KiB aligned by construction, so L3 always fits.
            .unwrap_or(Level::L3);

        let chunk = Chunk {
            level,
            va: self.va,
            pa: self.pa,
        };

        let size = level.entry_size();
        self.va += size;
        self.pa += size;
        self.remaining -= size;

        Some(chunk)
    }
}

/// Memory type of a mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemKind {
    /// Cacheable RAM.
    NormalWb,
    /// Device-nGnRnE MMIO: never cacheable, never executable.
    Device,
}

/// Access permissions of a mapping, at EL1 only for now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Perms {
    pub write: bool,
    pub execute: bool,
}

impl Perms {
    /// Read + write, no execute — data.
    pub const RW: Self = Self {
        write: true,
        execute: false,
    };
    /// Read + execute, no write — code.
    pub const RX: Self = Self {
        write: false,
        execute: true,
    };
    /// Read only.
    pub const RO: Self = Self {
        write: false,
        execute: false,
    };
}

/// Encode a level-1 block descriptor mapping `pa`.
///
/// Returns `None` if `pa` is not 1 GiB aligned or does not fit the 48-bit
/// output address, rather than silently masking the offending bits.
pub const fn l1_block(pa: u64, kind: MemKind, perms: Perms) -> Option<u64> {
    if pa % L1_BLOCK_SIZE != 0 {
        return None;
    }
    // The descriptor carries a 48-bit output address.
    if pa >= (1 << 48) {
        return None;
    }

    let mut desc = (pa & 0x0000_FFFF_C000_0000) | DESC_AF | DESC_SH_IS | DESC_BLOCK;

    desc |= match kind {
        MemKind::NormalWb => ATTR_IDX_NORMAL,
        MemKind::Device => ATTR_IDX_DEVICE,
    };

    desc |= if perms.write {
        DESC_AP_EL1_RW
    } else {
        DESC_AP_EL1_RO
    };

    // EL0 never executes a kernel mapping.
    desc |= DESC_UXN;
    if !perms.execute {
        desc |= DESC_PXN;
    }

    Some(desc)
}

/// Build `TCR_EL1` for a TTBR0-only kernel using the 4 KiB granule.
///
/// `t0sz` sets the TTBR0 virtual address size (`64 - t0sz` bits). TTBR1 is
/// disabled: with no upper-half mapping, a stray high address must fault
/// rather than start a walk through an uninitialised `TTBR1_EL1`.
pub const fn tcr_el1_ttbr0_only(t0sz: u64) -> u64 {
    t0sz
        | (0b01 << 8)      // IRGN0: inner write-back
        | (0b01 << 10)     // ORGN0: outer write-back
        | (0b11 << 12)     // SH0: inner shareable
        //   TG0 = 0b00 at [15:14]: 4 KiB granule
        | (25 << 16)       // T1SZ: legal value; unused because EPD1 is set
        | TCR_EPD1
        | (0b10u64 << 30)  // TG1 = 4 KiB: reserved encodings are avoided
        | (0b010u64 << 32) // IPS: 40-bit intermediate physical address
}

/// `TCR_EL1.EPD1` — disable translation-table walks via `TTBR1_EL1`.
pub const TCR_EPD1: u64 = 1 << 23;

/// Pages above which invalidating the whole TLB beats invalidating each entry.
///
/// `tlbi vaae1is` is precise but linear in the region: a 16 MiB mapping is 4096
/// operations, each broadcast to every core. Past some size the single
/// `tlbi vmalle1` is cheaper even counting the refills it forces. The exact
/// crossover is microarchitectural; this is a defensible round number, and the
/// choice is here rather than inline in the driver so it is visible and
/// testable instead of buried.
pub const TLBI_PAGE_LIMIT: u64 = 64;

/// How to invalidate the TLB after changing a mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlbiPlan {
    /// Invalidate each page: `tlbi vaae1is` with these operands.
    ByPage { first: u64, pages: u64 },
    /// Invalidate everything: `tlbi vmalle1`.
    Everything,
}

/// Decide how to invalidate the TLB for `[va, va + len)`.
///
/// `None` if the range is not page aligned — the caller has nothing sensible to
/// invalidate, and rounding would either miss entries or invalidate a
/// neighbour's.
pub const fn tlbi_plan(va: u64, len: u64) -> Option<TlbiPlan> {
    if va % PAGE_SIZE != 0 || len % PAGE_SIZE != 0 || len == 0 {
        return None;
    }

    let pages = len / PAGE_SIZE;
    if pages > TLBI_PAGE_LIMIT {
        return Some(TlbiPlan::Everything);
    }

    // `TLBI VAAE1IS` takes the virtual address shifted right by 12, not the
    // address itself. Passing the address invalidates a different page and
    // leaves a stale entry behind — which no boot would reveal, because the
    // stale entry is usually still correct.
    Some(TlbiPlan::ByPage {
        first: va >> 12,
        pages,
    })
}

/// Operand for `tlbi vaae1is` covering the page `index` pages after `first`.
pub const fn tlbi_operand(first: u64, index: u64) -> u64 {
    first + index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_block_is_cacheable_and_executable_at_el1_only() {
        let d = l1_block(0x4000_0000, MemKind::NormalWb, Perms::RX).unwrap();
        assert_eq!(d & 0x0000_FFFF_C000_0000, 0x4000_0000, "output address");
        assert_eq!(d & 0b11, DESC_BLOCK, "block descriptor");
        assert_ne!(d & DESC_AF, 0, "access flag must be set");
        assert_eq!(d & (0b111 << 2), ATTR_IDX_NORMAL, "MAIR index 0");
        assert_ne!(d & DESC_UXN, 0, "EL0 must never execute kernel memory");
        assert_eq!(d & DESC_PXN, 0, "executable at EL1");
    }

    #[test]
    fn device_block_is_never_executable() {
        let d = l1_block(0xC000_0000, MemKind::Device, Perms::RW).unwrap();
        assert_eq!(d & (0b111 << 2), ATTR_IDX_DEVICE, "MAIR index 1");
        assert_ne!(d & DESC_PXN, 0, "MMIO must not be executable at EL1");
        assert_ne!(d & DESC_UXN, 0, "MMIO must not be executable at EL0");
    }

    #[test]
    fn write_permission_selects_the_access_permission_bits() {
        let rw = l1_block(0, MemKind::NormalWb, Perms::RW).unwrap();
        let ro = l1_block(0, MemKind::NormalWb, Perms::RO).unwrap();
        assert_eq!(rw & (0b11 << 6), DESC_AP_EL1_RW);
        assert_eq!(ro & (0b11 << 6), DESC_AP_EL1_RO);
    }

    /// Masking a misaligned physical address produces a descriptor that maps
    /// something other than what the caller asked for. Refuse instead.
    #[test]
    fn misaligned_block_address_is_rejected() {
        assert_eq!(l1_block(0x4000_1000, MemKind::NormalWb, Perms::RW), None);
        assert_eq!(l1_block(0x8000_0000 - 1, MemKind::Device, Perms::RW), None);
    }

    #[test]
    fn output_address_beyond_48_bits_is_rejected() {
        assert_eq!(
            l1_block(1 << 52, MemKind::NormalWb, Perms::RW),
            None,
            "output address does not fit the descriptor"
        );
    }

    #[test]
    fn mair_places_normal_at_index_0_and_device_at_index_1() {
        assert_eq!(mair_el1() & 0xFF, MAIR_NORMAL_WB);
        assert_eq!((mair_el1() >> 8) & 0xFF, MAIR_DEVICE_NGNRNE);
    }

    /// The kernel maps nothing in the upper half. Leaving `EPD1` clear means a
    /// stray high virtual address starts a page-table walk through whatever
    /// `TTBR1_EL1` happened to contain at reset.
    // --- multi-level descriptors -------------------------------------------

    /// The AArch64 trap: at levels 1 and 2 a leaf is `0b01` and `0b11` means
    /// "table", but at level 3 a *page* is `0b11` and `0b01` is invalid. A
    /// level-3 leaf written as a block is simply not mapped.
    #[test]
    fn level3_leaves_use_the_page_encoding_not_the_block_encoding() {
        let page = leaf(Level::L3, 0x8000, MemKind::NormalWb, Perms::RX).unwrap();
        assert_eq!(page & 0b11, DESC_PAGE, "L3 leaf must be 0b11");

        let block = leaf(Level::L2, 0x20_0000, MemKind::NormalWb, Perms::RX).unwrap();
        assert_eq!(block & 0b11, DESC_BLOCK, "L2 leaf must be 0b01");
    }

    #[test]
    fn leaf_rejects_addresses_not_aligned_to_its_level() {
        assert!(leaf(Level::L3, 0x1000, MemKind::NormalWb, Perms::RW).is_some());
        assert_eq!(leaf(Level::L2, 0x1000, MemKind::NormalWb, Perms::RW), None);
        assert_eq!(
            leaf(Level::L1, 0x20_0000, MemKind::NormalWb, Perms::RW),
            None
        );
    }

    #[test]
    fn leaf_carries_permissions_at_every_level() {
        for level in [Level::L1, Level::L2, Level::L3] {
            let pa = level.entry_size();
            let rx = leaf(level, pa, MemKind::NormalWb, Perms::RX).unwrap();
            let rw = leaf(level, pa, MemKind::NormalWb, Perms::RW).unwrap();

            assert_eq!(rx & DESC_PXN, 0, "{level:?}: RX must be executable at EL1");
            assert_eq!(
                rx & (0b11 << 6),
                DESC_AP_EL1_RO,
                "{level:?}: RX is read-only"
            );
            assert_ne!(rw & DESC_PXN, 0, "{level:?}: RW must not be executable");
            assert_eq!(
                rw & (0b11 << 6),
                DESC_AP_EL1_RW,
                "{level:?}: RW is writable"
            );
            assert_eq!(rx & ADDR_MASK, pa, "{level:?}: output address");
        }
    }

    #[test]
    fn table_descriptor_points_at_a_page_aligned_table() {
        let d = table_descriptor(0x9_1000).unwrap();
        assert_eq!(d & 0b11, DESC_TABLE);
        assert_eq!(d & ADDR_MASK, 0x9_1000);
        assert_eq!(table_descriptor(0x9_1800), None, "must be page aligned");
    }

    #[test]
    fn level_indices_decode_the_virtual_address_fields() {
        // L1 index 1, L2 index 2, L3 index 3.
        let va = (1 << 30) | (2 << 21) | (3 << 12);
        assert_eq!(Level::L1.index(va), 1);
        assert_eq!(Level::L2.index(va), 2);
        assert_eq!(Level::L3.index(va), 3);
        // Indices wrap within their own 9 bits.
        assert_eq!(Level::L2.index(511 << 21), 511);
        assert_eq!(Level::L2.index(512 << 21), 0);
    }

    // --- region splitting --------------------------------------------------

    #[test]
    fn an_aligned_2mib_region_is_one_l2_block() {
        let c: Vec<_> = chunks(0x20_0000, 0x20_0000, L2_BLOCK_SIZE)
            .unwrap()
            .collect();
        assert_eq!(
            c,
            vec![Chunk {
                level: Level::L2,
                va: 0x20_0000,
                pa: 0x20_0000
            }]
        );
    }

    #[test]
    fn a_single_page_is_one_l3_page() {
        let c: Vec<_> = chunks(0x8000, 0x8000, PAGE_SIZE).unwrap().collect();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].level, Level::L3);
    }

    /// The kernel image starts at 0x80000, which is not 2 MiB aligned: the
    /// mapping has to page in up to the boundary before it can use blocks.
    #[test]
    fn an_unaligned_region_pages_in_then_blocks_then_pages_out() {
        let start = 0x8_0000;
        let len = 4 * L2_BLOCK_SIZE;
        let c: Vec<_> = chunks(start, start, len).unwrap().collect();

        assert_eq!(c[0].level, Level::L3, "must start with pages");
        assert!(
            c.iter().any(|k| k.level == Level::L2),
            "must use blocks in the middle"
        );

        // Contiguous, exactly covering, VA and PA staying in step.
        let mut cursor = start;
        for k in &c {
            assert_eq!(k.va, cursor);
            assert_eq!(k.pa, cursor);
            cursor += k.size();
        }
        assert_eq!(cursor, start + len);
    }

    /// Both addresses must be aligned: using a level on VA alignment alone
    /// would map physical memory the caller never asked for.
    #[test]
    fn mismatched_va_and_pa_alignment_falls_back_to_pages() {
        let c: Vec<_> = chunks(0x20_0000, 0x21_1000, 2 * L2_BLOCK_SIZE)
            .unwrap()
            .collect();
        assert!(c.iter().all(|k| k.level == Level::L3));
        assert_eq!(c.len() as u64, 2 * L2_BLOCK_SIZE / PAGE_SIZE);
    }

    #[test]
    fn a_gib_aligned_region_uses_l1_blocks() {
        let c: Vec<_> = chunks(0, 0, L1_BLOCK_SIZE).unwrap().collect();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].level, Level::L1);
    }

    #[test]
    fn an_empty_region_yields_nothing() {
        assert_eq!(chunks(0x1000, 0x1000, 0).unwrap().count(), 0);
    }

    #[test]
    fn unaligned_requests_are_rejected_rather_than_rounded() {
        assert!(chunks(0x1800, 0x1000, PAGE_SIZE).is_none());
        assert!(chunks(0x1000, 0x1800, PAGE_SIZE).is_none());
        assert!(chunks(0x1000, 0x1000, 0x800).is_none());
    }

    // --- TLB invalidation planning ------------------------------------------

    /// The operand is the address shifted right by 12, not the address. Getting
    /// this wrong invalidates some other page and leaves a stale entry that is
    /// usually still correct — so nothing visibly breaks until it does.
    #[test]
    fn by_page_operands_are_the_address_shifted_by_twelve() {
        let plan = tlbi_plan(0x4_2000, 2 * PAGE_SIZE).unwrap();
        assert_eq!(
            plan,
            TlbiPlan::ByPage {
                first: 0x4_2000 >> 12,
                pages: 2
            }
        );
        let TlbiPlan::ByPage { first, .. } = plan else {
            unreachable!()
        };
        assert_eq!(tlbi_operand(first, 0), 0x42);
        assert_eq!(tlbi_operand(first, 1), 0x43);
    }

    #[test]
    fn one_page_is_one_operand() {
        assert_eq!(
            tlbi_plan(0x1000, PAGE_SIZE),
            Some(TlbiPlan::ByPage { first: 1, pages: 1 })
        );
    }

    #[test]
    fn a_large_region_invalidates_everything() {
        let len = (TLBI_PAGE_LIMIT + 1) * PAGE_SIZE;
        assert_eq!(tlbi_plan(0x1000, len), Some(TlbiPlan::Everything));
    }

    /// Exactly at the limit stays precise: the threshold is "more than", so a
    /// region of exactly the limit does not tip over into a global flush.
    #[test]
    fn the_limit_itself_stays_per_page() {
        let len = TLBI_PAGE_LIMIT * PAGE_SIZE;
        assert_eq!(
            tlbi_plan(0x1000, len),
            Some(TlbiPlan::ByPage {
                first: 1,
                pages: TLBI_PAGE_LIMIT
            })
        );
    }

    #[test]
    fn an_unaligned_or_empty_range_has_no_plan() {
        assert_eq!(tlbi_plan(0x1800, PAGE_SIZE), None);
        assert_eq!(tlbi_plan(0x1000, 0x800), None);
        assert_eq!(tlbi_plan(0x1000, 0), None);
    }

    // --- leaf decode (needed to split blocks on unmap) ----------------------

    #[test]
    fn decode_round_trips_every_permission_and_kind() {
        for level in [Level::L1, Level::L2, Level::L3] {
            let pa = match level {
                Level::L1 => 0x4000_0000,
                Level::L2 => 0x0020_0000,
                Level::L3 => 0x0000_3000,
            };
            for kind in [MemKind::NormalWb, MemKind::Device] {
                for perms in [Perms::RW, Perms::RX, Perms::RO] {
                    let desc = leaf(level, pa, kind, perms).expect("aligned pa");
                    assert!(is_leaf(desc, level));
                    assert!(!is_table(desc, level));
                    assert!(!is_invalid(desc));
                    let (got_pa, got_kind, got_perms) =
                        decode_leaf(desc, level).expect("decode leaf");
                    assert_eq!(got_pa, pa);
                    assert_eq!(got_kind, kind);
                    assert_eq!(got_perms, perms);
                }
            }
        }
    }

    #[test]
    fn decode_rejects_invalid_and_table_entries() {
        assert!(decode_leaf(0, Level::L3).is_none());
        let table = table_descriptor(0x9000).unwrap();
        assert!(is_table(table, Level::L2));
        assert!(decode_leaf(table, Level::L2).is_none());
        // At L3 the same bits are a page at PA 0x9000, not a table.
        assert!(is_leaf(table, Level::L3));
    }

    #[test]
    fn tcr_disables_ttbr1_walks() {
        let tcr = tcr_el1_ttbr0_only(25);
        assert_ne!(tcr & TCR_EPD1, 0, "EPD1 must be set when TTBR1 is unused");
    }

    #[test]
    fn tcr_encodes_the_documented_translation_regime() {
        let tcr = tcr_el1_ttbr0_only(25);
        assert_eq!(tcr & 0x3F, 25, "T0SZ → 39-bit VA");
        assert_eq!((tcr >> 8) & 0b11, 0b01, "IRGN0 write-back");
        assert_eq!((tcr >> 10) & 0b11, 0b01, "ORGN0 write-back");
        assert_eq!((tcr >> 12) & 0b11, 0b11, "SH0 inner shareable");
        assert_eq!((tcr >> 14) & 0b11, 0b00, "TG0 4 KiB granule");
        assert_eq!((tcr >> 32) & 0b111, 0b010, "IPS 40-bit");
    }
}
