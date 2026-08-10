//! `ESR_EL1` in words.
//!
//! A fatal trap on silicon prints its syndrome and nothing else: there is no
//! debugger on the board, so the hex *is* the diagnosis, and reading it means
//! having the ARM ARM open at the exception-class table. That translation is
//! total, boring and easy to get subtly wrong — which is exactly the shape
//! that belongs here rather than in the handler (same split as [`crate::cpuid`]
//! and [`crate::reset`], ADR-0065).
//!
//! Unknown encodings are named as unknown and carry their value. A decoder
//! that guesses would turn "I have never seen this" into a plausible sentence,
//! which is worse than hex.

/// Exception class (`ESR_EL1.EC`, bits 31:26) and, for aborts, the fault
/// status its ISS carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fault {
    /// What kind of exception this is.
    pub class: &'static str,
    /// Why it aborted, when the class is an abort. `None` otherwise — an
    /// absent detail is not an unknown one.
    pub detail: Option<Detail>,
    /// True when `FAR_EL1` holds a meaningful address for this class.
    pub far_valid: bool,
}

/// Fault status decoded from `DFSC`/`IFSC` (ISS bits 5:0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detail {
    pub kind: &'static str,
    /// Translation-table level, where the status encodes one.
    pub level: Option<u8>,
    /// Set for a data abort caused by a write rather than a read (`WnR`).
    pub write: Option<bool>,
}

/// Decode `ESR_EL1`.
pub fn describe(esr: u64) -> Fault {
    let ec = (esr >> 26) & 0x3f;
    let iss = esr & 0x01ff_ffff;
    let (class, is_abort, far_valid) = match ec {
        0x00 => ("unknown reason", false, false),
        0x0e => ("illegal execution state", false, false),
        0x15 => ("SVC from AArch64", false, false),
        0x18 => ("trapped MSR/MRS or system instruction", false, false),
        0x20 => ("instruction abort, lower EL", true, true),
        0x21 => ("instruction abort, current EL", true, true),
        0x22 => ("PC alignment fault", false, true),
        0x24 => ("data abort, lower EL", true, true),
        0x25 => ("data abort, current EL", true, true),
        0x26 => ("SP alignment fault", false, false),
        0x2c => ("floating-point exception", false, false),
        0x30 | 0x31 => ("breakpoint", false, true),
        0x32 | 0x33 => ("software step", false, true),
        0x34 | 0x35 => ("watchpoint", false, true),
        0x3c => ("BRK instruction", false, false),
        _ => ("unrecognised exception class", false, false),
    };
    let detail = if is_abort {
        Some(abort_detail(iss, ec == 0x24 || ec == 0x25))
    } else {
        None
    };
    Fault {
        class,
        detail,
        far_valid,
    }
}

/// `DFSC`/`IFSC` (ISS bits 5:0), plus `WnR` (bit 6) for data aborts.
fn abort_detail(iss: u64, is_data: bool) -> Detail {
    let status = (iss & 0x3f) as u8;
    let level = |bits: u8| Some(bits & 0b11);
    let (kind, level) = match status {
        0x00..=0x03 => ("address size fault", level(status)),
        0x04..=0x07 => ("translation fault", level(status)),
        0x08..=0x0b => ("access flag fault", level(status)),
        0x0c..=0x0f => ("permission fault", level(status)),
        0x10 => ("synchronous external abort", None),
        0x11 => ("synchronous tag check fault", None),
        0x14..=0x17 => ("synchronous external abort on table walk", level(status)),
        0x18 => ("synchronous parity or ECC error", None),
        0x1c..=0x1f => (
            "synchronous parity or ECC error on table walk",
            level(status),
        ),
        0x21 => ("alignment fault", None),
        0x30 => ("TLB conflict abort", None),
        _ => ("unrecognised fault status", None),
    };
    Detail {
        kind,
        level,
        // `WnR` is only defined for data aborts, and only when the fault is
        // not on a table walk of an instruction fetch.
        write: if is_data {
            Some(iss & (1 << 6) != 0)
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el0_write_to_a_kernel_page_is_a_permission_fault() {
        // The `el0: FAULT ok` gate prints this exact syndrome every boot, on
        // QEMU and on silicon. Reading it by hand is what this module removes:
        // the address translates fine — EL0 simply may not write there — and
        // "translation fault" was this author's first guess at the hex.
        let f = describe(0x9200_004f);
        assert_eq!(f.class, "data abort, lower EL");
        assert!(f.far_valid);
        let d = f.detail.unwrap();
        assert_eq!(d.kind, "permission fault");
        assert_eq!(d.level, Some(3));
        assert_eq!(d.write, Some(true));
    }

    #[test]
    fn current_el_permission_fault_read() {
        // EC=0x25, DFSC=0x0d (permission fault, level 1), WnR clear.
        let f = describe((0x25 << 26) | 0x0d);
        assert_eq!(f.class, "data abort, current EL");
        let d = f.detail.unwrap();
        assert_eq!(d.kind, "permission fault");
        assert_eq!(d.level, Some(1));
        assert_eq!(d.write, Some(false));
    }

    #[test]
    fn external_abort_is_the_probe_signature() {
        // What `arch::probe` recognises: EC=0x25, DFSC=0x10, no level.
        let d = describe((0x25 << 26) | 0x10).detail.unwrap();
        assert_eq!(d.kind, "synchronous external abort");
        assert_eq!(d.level, None);
    }

    #[test]
    fn instruction_abort_has_no_write_bit() {
        let f = describe((0x21 << 26) | 0x07);
        assert_eq!(f.class, "instruction abort, current EL");
        let d = f.detail.unwrap();
        assert_eq!(d.kind, "translation fault");
        assert_eq!(d.write, None);
    }

    #[test]
    fn svc_has_no_abort_detail_and_no_address() {
        let f = describe(0x5600_0000);
        assert_eq!(f.class, "SVC from AArch64");
        assert!(f.detail.is_none());
        assert!(!f.far_valid);
    }

    #[test]
    fn unknown_class_says_so_rather_than_guessing() {
        let f = describe(0x2c << 26);
        assert_eq!(f.class, "floating-point exception");
        let f = describe(0x1f << 26);
        assert_eq!(f.class, "unrecognised exception class");
    }
}
