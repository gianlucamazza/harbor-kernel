//! Hardware description and reconciliation — ADR-0072's evidence spine.
//!
//! A [`HwDescription`] is what a firmware table said; [`CompiledClaims`] is
//! what the BSP compiled in. [`reconcile`] compares the two and yields one
//! [`Verdict`] per fact. Nothing here configures anything: the output is a
//! report the boot prints and the oracle asserts ("verify, don't select").
//!
//! Board-free by construction: addresses and expectations arrive as
//! arguments, so this module never names a physical address.

use crate::fdt::{Extract, MemRange, Str64};

/// Where an observed fact came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A compile-time claim (e.g. a cargo feature), reported as such.
    Compiled,
    /// A CPU identification register.
    IdRegister,
    /// A firmware-provided table.
    FirmwareTable(TableKind),
    /// An MMIO presence probe (`arch::probe` style).
    Probe,
}

/// Which firmware table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    Fdt,
    Cpuid,
    PvhMemMap,
}

/// Outcome of comparing one observed fact with its compiled claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Observation agrees with the compiled claim.
    Matches,
    /// Hardware reports more than the compiled map uses — the evidence line
    /// a future identity-map-raise ADR must cite (ADR-0072 §2).
    BeyondCompiledMap,
    /// Hardware reports less than the compiled map assumes.
    Short,
    /// Observation and claim disagree (non-memory facts).
    Differs,
    /// Nothing usable was observed; the reason is reported alongside.
    Unknown,
}

/// What a firmware table described. Built from [`Extract`] today; the x86
/// lab slice will build it from CPUID/PVH instead (same shape, same report).
#[derive(Debug, Clone, Copy)]
pub struct HwDescription {
    pub source: TableKind,
    pub model: Str64,
    pub revision: Option<u32>,
    pub memory: [MemRange; crate::fdt::MAX_MEM_RANGES],
    pub memory_ranges: usize,
    pub memory_total: u64,
    pub cpus: u32,
}

impl HwDescription {
    pub fn from_fdt(x: &Extract) -> Self {
        Self {
            source: TableKind::Fdt,
            model: x.model,
            revision: x.revision,
            memory: x.memory,
            memory_ranges: x.memory_ranges,
            memory_total: x.memory_total,
            cpus: x.cpus,
        }
    }
}

/// The compiled expectations the report reconciles against.
#[derive(Debug, Clone, Copy)]
pub struct CompiledClaims {
    /// Exclusive end of the identity-mapped RAM the kernel actually uses.
    pub identity_ram_end: u64,
    /// Cores the BSP expects the SoC to have.
    pub expected_cpus: u32,
    /// The model string must start with this.
    pub model_prefix: &'static str,
}

/// One verdict per fact.
#[derive(Debug, Clone, Copy)]
pub struct Report {
    pub model: Verdict,
    pub memory: Verdict,
    pub cpus: Verdict,
}

/// Pure reconciliation. Memory compares with `>=`, never equality: the
/// tree reports ARM memory (the VideoCore share is excluded), and a zero
/// total is an un-patched distributed blob, reported as [`Verdict::Unknown`]
/// rather than as a short board.
pub fn reconcile(claims: &CompiledClaims, observed: &HwDescription) -> Report {
    let model = if observed.model.is_empty() {
        Verdict::Unknown
    } else if observed.model.as_str().starts_with(claims.model_prefix) {
        Verdict::Matches
    } else {
        Verdict::Differs
    };

    let memory = if observed.memory_total == 0 {
        Verdict::Unknown
    } else if observed.memory_total > claims.identity_ram_end {
        Verdict::BeyondCompiledMap
    } else if observed.memory_total == claims.identity_ram_end {
        Verdict::Matches
    } else {
        Verdict::Short
    };

    let cpus = if observed.cpus == 0 {
        Verdict::Unknown
    } else if observed.cpus == claims.expected_cpus {
        Verdict::Matches
    } else {
        Verdict::Differs
    };

    Report {
        model,
        memory,
        cpus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fdt;

    const PI4: &[u8] = include_bytes!("../tests/fixtures/bcm2711-rpi-4-b.dtb");

    fn claims() -> CompiledClaims {
        CompiledClaims {
            identity_ram_end: 0x8000_0000,
            expected_cpus: 4,
            model_prefix: "Raspberry Pi 4 Model B",
        }
    }

    #[test]
    fn distributed_blob_reconciles_honestly() {
        let x = fdt::extract(PI4).unwrap();
        let hw = HwDescription::from_fdt(&x);
        let r = reconcile(&claims(), &hw);
        assert_eq!(r.model, Verdict::Matches);
        // Zero-size memory node: unknown, never "short".
        assert_eq!(r.memory, Verdict::Unknown);
        assert_eq!(r.cpus, Verdict::Matches);
    }

    #[test]
    fn four_gib_board_exceeds_two_gib_map() {
        let x = fdt::extract(PI4).unwrap();
        let mut hw = HwDescription::from_fdt(&x);
        hw.memory_total = 4 << 30;
        assert_eq!(reconcile(&claims(), &hw).memory, Verdict::BeyondCompiledMap);
    }

    #[test]
    fn exact_and_short_memory() {
        let x = fdt::extract(PI4).unwrap();
        let mut hw = HwDescription::from_fdt(&x);
        hw.memory_total = 0x8000_0000;
        assert_eq!(reconcile(&claims(), &hw).memory, Verdict::Matches);
        hw.memory_total = 1 << 30;
        assert_eq!(reconcile(&claims(), &hw).memory, Verdict::Short);
    }

    #[test]
    fn wrong_board_and_cpu_count_differ() {
        let x = fdt::extract(PI4).unwrap();
        let mut hw = HwDescription::from_fdt(&x);
        hw.cpus = 2;
        let mut c = claims();
        c.model_prefix = "Raspberry Pi 5";
        let r = reconcile(&c, &hw);
        assert_eq!(r.model, Verdict::Differs);
        assert_eq!(r.cpus, Verdict::Differs);
    }
}
