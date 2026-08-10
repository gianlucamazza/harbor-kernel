//! CPU identity decode (ADR-0065) — the platform self-check's arithmetic.
//!
//! The kernel is built on knowledge about the core it runs on — exclusives
//! that make no progress on Device memory before the MMU, an I-cache that is
//! not coherent with the D-cache, ADR-0050's ASID arithmetic — and until
//! ADR-0065 none of it was observed at runtime: "Cortex-A72" existed only in
//! comments. This module turns the ID registers into typed answers so the
//! boot can print what it is actually running on and refuse when a
//! load-bearing assumption does not hold.
//!
//! Same split as [`crate::reset`]: the arch layer reads the registers
//! (`mrs`, one per register, no logic), this module owns the decode as total
//! functions over integers, host-tested. Field layouts follow the Arm ARM
//! (DDI 0487) descriptions of `MIDR_EL1`, `ID_AA64MMFR0_EL1` and
//! `ID_AA64PFR0_EL1`.
//!
//! Reserved encodings decode to `None` / unsupported rather than to a
//! plausible value — the same refusal to manufacture an answer that
//! [`crate::reset::ResetCause::None`] exists for.

/// The part the kernel recognises, or the raw identity when it does not.
///
/// One entry deep on purpose: the product is a single SKU (Pi 4B, Cortex-A72)
/// and a recognised-parts table with speculative rows would be dispatch with
/// no consumer. An unknown part is carried whole, never collapsed — the boot
/// line prints it, and porting a second core starts by adding its row here
/// (`docs/porting.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// Arm Cortex-A72 (implementer `0x41`, part `0xD08`) — the BCM2711 core.
    CortexA72,
    /// Anything else, kept as the raw implementer and part fields.
    Unknown { implementer: u8, part: u16 },
}

/// Arm Limited's implementer code in `MIDR_EL1[31:24]`.
pub const IMPLEMENTER_ARM: u8 = 0x41;
/// Cortex-A72 part number in `MIDR_EL1[15:4]`.
pub const PART_CORTEX_A72: u16 = 0xD08;

/// Decode `MIDR_EL1` into the part the kernel recognises.
///
/// ```
/// use kernel_core::cpuid::{Part, part};
///
/// // The BCM2711 core, as QEMU's `-cpu cortex-a72` also reports it.
/// assert_eq!(part(0x410F_D083), Part::CortexA72);
///
/// // An unrecognised core keeps its whole identity for the boot line.
/// assert_eq!(
///     part(0x410F_D0B3),
///     Part::Unknown { implementer: 0x41, part: 0xD0B }
/// );
/// ```
#[inline]
pub const fn part(midr: u64) -> Part {
    let implementer = ((midr >> 24) & 0xFF) as u8;
    let part = ((midr >> 4) & 0xFFF) as u16;
    match (implementer, part) {
        (IMPLEMENTER_ARM, PART_CORTEX_A72) => Part::CortexA72,
        _ => Part::Unknown { implementer, part },
    }
}

/// Major silicon revision — the `N` of `rNpM`, from `MIDR_EL1[23:20]`.
#[inline]
pub const fn variant(midr: u64) -> u8 {
    ((midr >> 20) & 0xF) as u8
}

/// Minor silicon revision — the `M` of `rNpM`, from `MIDR_EL1[3:0]`.
#[inline]
pub const fn revision(midr: u64) -> u8 {
    (midr & 0xF) as u8
}

/// Hardware ASID width in bits from `ID_AA64MMFR0_EL1.ASIDBits`.
///
/// The values the architecture defines are 8 and 16; everything else in the
/// field is reserved and decodes to `None`. This is the number ADR-0050's
/// isolation depends on: the pool hands out `asid::ASID_BITS`-bit values, and
/// hardware that implements fewer bits than the pool assumes would alias two
/// address spaces in the TLB — silently.
///
/// ```
/// use kernel_core::cpuid::asid_bits;
///
/// // Cortex-A72 (`ID_AA64MMFR0_EL1 = 0x1124`): 16-bit ASIDs.
/// assert_eq!(asid_bits(0x1124), Some(16));
/// // ASIDBits = 0b0000 is the 8-bit floor every ARMv8 core may report.
/// assert_eq!(asid_bits(0x1104), Some(8));
/// // A reserved encoding is not an answer.
/// assert_eq!(asid_bits(0x1114), None);
/// ```
#[inline]
pub const fn asid_bits(mmfr0: u64) -> Option<u32> {
    match (mmfr0 >> 4) & 0xF {
        0b0000 => Some(8),
        0b0010 => Some(16),
        _ => None,
    }
}

/// Physical address range in bits from `ID_AA64MMFR0_EL1.PARange`.
///
/// Reserved encodings decode to `None`. Diagnostic on the boot line rather
/// than load-bearing: the kernel's own map is far below the 44-bit floor of
/// any core it could plausibly boot on.
#[inline]
pub const fn pa_bits(mmfr0: u64) -> Option<u32> {
    match mmfr0 & 0xF {
        0b0000 => Some(32),
        0b0001 => Some(36),
        0b0010 => Some(40),
        0b0011 => Some(42),
        0b0100 => Some(44),
        0b0101 => Some(48),
        0b0110 => Some(52),
        _ => None,
    }
}

/// Whether the 4 KiB translation granule is implemented, from
/// `ID_AA64MMFR0_EL1.TGran4`.
///
/// `0b0000` is plain support and `0b0001` is support with 52-bit addresses
/// (FEAT_LPA2); `0b1111` is "not implemented" and every other value is
/// reserved, which this decode refuses to call support. The whole paging
/// model — `paging`, the table arena, every `UserWindow` — is written against
/// this granule, so a core without it cannot run this kernel at all.
#[inline]
pub const fn tgran4_supported(mmfr0: u64) -> bool {
    matches!((mmfr0 >> 28) & 0xF, 0b0000 | 0b0001)
}

/// Whether EL0 is implemented in AArch64, from `ID_AA64PFR0_EL1.EL0`.
///
/// `0b0001` is AArch64 only, `0b0010` is AArch64 with AArch32 as well; both
/// answer yes. The EL0 session model (ADR-0017/0023) is meaningless without
/// it.
#[inline]
pub const fn el0_aarch64(pfr0: u64) -> bool {
    matches!(pfr0 & 0xF, 0b0001 | 0b0010)
}

/// Whether EL1 is implemented in AArch64, from `ID_AA64PFR0_EL1.EL1`.
#[inline]
pub const fn el1_aarch64(pfr0: u64) -> bool {
    matches!((pfr0 >> 4) & 0xF, 0b0001 | 0b0010)
}

/// Whether FP/AdvSIMD is implemented, from `ID_AA64PFR0_EL1.FP`.
///
/// Not asserted anywhere: ADR-0002 compiles the kernel softfloat and leaves
/// FP trapping, so the kernel is correct either way. Decoded because the
/// premise "there is an FP unit we chose not to use" is part of ADR-0002's
/// context, and a transcript that can show it is worth one field.
#[inline]
pub const fn fp_implemented(pfr0: u64) -> bool {
    matches!((pfr0 >> 16) & 0xF, 0b0000 | 0b0001)
}

// --- x86 CPUID leaf 1 (eax) — pure decode for lab / host-class path (ADR-0065) ---

/// Display family from CPUID leaf 1 `eax` (Intel SDM Vol. 2A / AMD APM).
///
/// Base family in bits [11:8]; when base is `0x0F`, add extended family [27:20].
#[inline]
#[must_use]
pub const fn x86_display_family(leaf1_eax: u32) -> u32 {
    let base = (leaf1_eax >> 8) & 0xf;
    let ext = (leaf1_eax >> 20) & 0xff;
    if base == 0x0f {
        base + ext
    } else {
        base
    }
}

/// Display model from CPUID leaf 1 `eax`.
///
/// Base model in bits [7:4]; when base family is `0x06` or `0x0F`, add
/// extended model [19:16] in the high nibble.
#[inline]
#[must_use]
pub const fn x86_display_model(leaf1_eax: u32) -> u32 {
    let base_family = (leaf1_eax >> 8) & 0xf;
    let base_model = (leaf1_eax >> 4) & 0xf;
    let ext_model = (leaf1_eax >> 16) & 0xf;
    if base_family == 0x06 || base_family == 0x0f {
        (ext_model << 4) | base_model
    } else {
        base_model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registers as the Cortex-A72 in the Pi 4B (and QEMU's model of it)
    /// reports them — the values the boot line and the oracle assert on.
    const A72_MIDR: u64 = 0x410F_D083;
    const A72_MMFR0: u64 = 0x0000_1124;
    const A72_PFR0: u64 = 0x0000_2222;

    #[test]
    fn the_bcm2711_core_is_recognised_with_its_revision() {
        assert_eq!(part(A72_MIDR), Part::CortexA72);
        // r0p3, the stepping every Pi 4B ships.
        assert_eq!(variant(A72_MIDR), 0);
        assert_eq!(revision(A72_MIDR), 3);
    }

    #[test]
    fn an_unknown_part_keeps_its_whole_identity() {
        // A Cortex-A76 (0xD0B) must not decode to "some A72": the boot line
        // prints exactly what was found, and collapsing the fields would
        // manufacture a recognition that did not happen.
        let midr = 0x414F_D0B1;
        assert_eq!(
            part(midr),
            Part::Unknown {
                implementer: 0x41,
                part: 0xD0B
            }
        );
        assert_eq!(variant(midr), 4);
        assert_eq!(revision(midr), 1);
    }

    #[test]
    fn the_a72_reports_the_platform_the_kernel_assumes() {
        // The exact conjunction bootstrap refuses to boot without —
        // one place where the compiled expectation is spelled out in full.
        assert_eq!(asid_bits(A72_MMFR0), Some(16));
        assert_eq!(pa_bits(A72_MMFR0), Some(44));
        assert!(tgran4_supported(A72_MMFR0));
        assert!(el0_aarch64(A72_PFR0));
        assert!(el1_aarch64(A72_PFR0));
        assert!(fp_implemented(A72_PFR0));
    }

    #[test]
    fn eight_bit_asids_are_the_floor_and_reserved_is_no_answer() {
        assert_eq!(asid_bits(0x0000_1104), Some(8));
        // Every reserved ASIDBits encoding must stay `None`: a decode that
        // guessed "8" here could pass the width check on hardware whose real
        // width nobody knows.
        for reserved in [0b0001u64, 0b0011, 0b0111, 0b1111] {
            assert_eq!(asid_bits(reserved << 4), None, "ASIDBits {reserved:#b}");
        }
    }

    #[test]
    fn tgran4_reserved_encodings_are_not_support() {
        assert!(tgran4_supported(0x0000_0000)); // plain support
        assert!(tgran4_supported(0x1000_0000)); // support + LPA2
        assert!(!tgran4_supported(0xF000_0000)); // not implemented
        // Reserved must not be read as "supported": the paging model would
        // then be built on a field the architecture has not defined.
        assert!(!tgran4_supported(0x7000_0000));
    }

    #[test]
    fn missing_els_are_refused_not_defaulted() {
        // EL0/EL1 "not implemented in AArch64" (0b0000) and every reserved
        // value answer no; only the two defined yes-encodings answer yes.
        assert!(!el0_aarch64(0x0000_2220));
        assert!(!el1_aarch64(0x0000_2202));
        assert!(el0_aarch64(0x0000_0001));
        assert!(el1_aarch64(0x0000_0020));
    }

    #[test]
    fn an_fp_less_core_is_visible_as_such() {
        // FP = 0b1111 is "no FP/AdvSIMD". ADR-0002 keeps the kernel correct
        // on such a core; the decode keeps the transcript honest about it.
        assert!(!fp_implemented(0x000F_2222));
    }

    #[test]
    fn x86_leaf1_display_family_model_match_sdm() {
        // QEMU `-cpu qemu64` style: base family 0xF, ext family 6 → 15;
        // base model 0xB, ext model 0x6 → 0x6B (107). eax1 = 0x00060fb1.
        let eax = 0x0006_0fb1;
        assert_eq!(x86_display_family(eax), 15);
        assert_eq!(x86_display_model(eax), 107);

        // Family 6 without extended family: model uses ext model nibble.
        // eax = base family 6, base model 0xA, ext model 0x1 → model 0x1A.
        let eax6 = (0x6 << 8) | (0xa << 4) | (0x1 << 16);
        assert_eq!(x86_display_family(eax6), 6);
        assert_eq!(x86_display_model(eax6), 0x1a);
    }
}
