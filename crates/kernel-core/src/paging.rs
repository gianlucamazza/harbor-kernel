//! AArch64 stage-1 translation encodings (4 KiB granule).
//!
//! Descriptor and `TCR_EL1` bit layouts, isolated from the `msr` sequence that
//! installs them. A wrong bit here is a silent fault or a walk through an
//! uninitialised table, neither of which is observable from a serial log.

/// Block descriptor at level 1 or 2.
pub const DESC_BLOCK: u64 = 0b01;
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
