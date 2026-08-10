//! GICv2 register index arithmetic.
//!
//! The distributor exposes several arrays indexed by interrupt id: one bit per
//! id (enable, pending, group) or one byte per id (priority). Getting the word
//! offset or the shift wrong silently touches a neighbouring interrupt, which
//! is exactly the class of bug that never shows up until a second IRQ exists.

/// Id reported by `GICC_IAR` when there is nothing to claim.
pub const SPURIOUS_ID: u32 = 1023;

/// Id reported when the pending interrupt belongs to the other security group.
pub const SPURIOUS_GROUP_ID: u32 = 1022;

/// Byte offset (from the array base) and mask for a one-bit-per-id array:
/// `ISENABLER`, `ICENABLER`, `ICPENDR`, `IGROUPR`.
pub const fn bit_slot(irq: u32) -> (usize, u32) {
    let word = (irq / 32) as usize;
    let bit = irq % 32;
    (word * 4, 1u32 << bit)
}

/// Byte offset and bit shift for a one-byte-per-id array: `IPRIORITYR`,
/// `ITARGETSR`.
pub const fn byte_slot(irq: u32) -> (usize, u32) {
    let word = (irq / 4) as usize;
    let shift = (irq % 4) * 8;
    (word * 4, shift)
}

/// Replace the byte belonging to `irq` inside a read-modify-write word.
pub const fn insert_byte(word: u32, irq: u32, value: u8) -> u32 {
    let (_, shift) = byte_slot(irq);
    (word & !(0xFF << shift)) | ((value as u32) << shift)
}

/// Interrupt id carried by an `IAR`/`EOIR` word (the upper bits hold the
/// CPU id and must be preserved when writing `EOIR` back).
pub const fn ack_id(raw: u32) -> u32 {
    raw & 0x3FF
}

/// True when an `IAR` word carries no real interrupt.
pub const fn is_spurious(raw: u32) -> bool {
    let id = ack_id(raw);
    id == SPURIOUS_ID || id == SPURIOUS_GROUP_ID
}

/// Interrupt id classes, which decide whether an id is banked per-CPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqClass {
    /// 0–15: software generated, banked per CPU.
    Sgi,
    /// 16–31: private peripheral, banked per CPU (the arch timer lives here).
    Ppi,
    /// 32–1019: shared peripheral, needs explicit CPU targeting.
    Spi,
    /// Everything else is not a deliverable interrupt id.
    Invalid,
}

/// Classify an interrupt id.
pub const fn classify(irq: u32) -> IrqClass {
    if irq < 16 {
        IrqClass::Sgi
    } else if irq < 32 {
        IrqClass::Ppi
    } else if irq < 1020 {
        IrqClass::Spi
    } else {
        IrqClass::Invalid
    }
}

/// How `GICD_SGIR.TargetListFilter` picks the set of CPUs that receive an SGI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SgiFilter {
    /// Forward to the CPUs named in `CPUTargetList` (one bit per interface).
    TargetList = 0b00,
    /// All interfaces except the requester.
    AllButSelf = 0b01,
    /// The requesting CPU only.
    SelfOnly = 0b10,
}

/// Encode a GICv2 `GICD_SGIR` write (ADR-0074).
///
/// - `sgi_id` is 0…15 (software-generated id).
/// - `cpu_target_list` is the 8-bit target mask used when `filter` is
///   [`SgiFilter::TargetList`]; ignored for the other filters but still
///   placed in the word so a host test can pin the field layout.
///
/// Returns `None` if `sgi_id` is not an SGI.
pub const fn sgir_word(sgi_id: u32, cpu_target_list: u8, filter: SgiFilter) -> Option<u32> {
    if sgi_id > 15 {
        return None;
    }
    let word = (filter as u32) << 24 | (cpu_target_list as u32) << 16 | sgi_id;
    Some(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_slot_picks_word_and_bit() {
        assert_eq!(bit_slot(0), (0, 1 << 0));
        // PPI 30: still word 0.
        assert_eq!(bit_slot(30), (0, 1 << 30));
        assert_eq!(bit_slot(31), (0, 1 << 31));
        // First id of the second word.
        assert_eq!(bit_slot(32), (4, 1 << 0));
        assert_eq!(bit_slot(97), (12, 1 << 1));
    }

    #[test]
    fn byte_slot_picks_word_and_shift() {
        assert_eq!(byte_slot(0), (0, 0));
        assert_eq!(byte_slot(3), (0, 24));
        assert_eq!(byte_slot(4), (4, 0));
        assert_eq!(byte_slot(30), (28, 16));
    }

    #[test]
    fn insert_byte_leaves_neighbours_untouched() {
        let word = 0xAA_BB_CC_DD;
        // irq 30 sits at shift 16 inside its word.
        assert_eq!(insert_byte(word, 30, 0x00), 0xAA_00_CC_DD);
        assert_eq!(insert_byte(word, 28, 0x11), 0xAA_BB_CC_11);
        assert_eq!(insert_byte(word, 31, 0x22), 0x22_BB_CC_DD);
    }

    #[test]
    fn ack_preserves_cpu_id_but_reports_the_interrupt() {
        // CPU id 2 in bits [12:10], interrupt 30.
        let raw = (2 << 10) | 30;
        assert_eq!(ack_id(raw), 30);
        assert!(!is_spurious(raw));
    }

    #[test]
    fn both_spurious_encodings_are_recognised() {
        assert!(is_spurious(1023));
        assert!(is_spurious(1022));
        assert!(!is_spurious(1021));
    }

    #[test]
    fn classification_matches_the_gic_id_map() {
        assert_eq!(classify(0), IrqClass::Sgi);
        assert_eq!(classify(15), IrqClass::Sgi);
        assert_eq!(classify(16), IrqClass::Ppi);
        // The arch physical timer.
        assert_eq!(classify(30), IrqClass::Ppi);
        assert_eq!(classify(31), IrqClass::Ppi);
        assert_eq!(classify(32), IrqClass::Spi);
        assert_eq!(classify(1019), IrqClass::Spi);
        assert_eq!(classify(1020), IrqClass::Invalid);
        assert_eq!(classify(SPURIOUS_ID), IrqClass::Invalid);
    }

    #[test]
    fn sgir_word_targets_cpu1_with_sgi0() {
        // ADR-0074: primary wakes core 1 via SGI 0 + CPUTargetList bit 1.
        assert_eq!(
            sgir_word(0, 1 << 1, SgiFilter::TargetList),
            Some(0x0002_0000)
        );
    }

    #[test]
    fn sgir_word_refuses_non_sgi_ids() {
        assert_eq!(sgir_word(16, 0x01, SgiFilter::SelfOnly), None);
        assert_eq!(
            sgir_word(15, 0x00, SgiFilter::AllButSelf),
            Some((0b01 << 24) | 15)
        );
    }
}
