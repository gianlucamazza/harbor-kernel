//! Hardware self-discovery report (ADR-0072 / ADR-0073).
//!
//! Observe → pure decode (`kernel_core::{fdt,hwdesc}`) → reconcile against
//! compiled BSP claims → unconditional `discover:` lines. Never selects
//! configuration (verify, don't select).
//!
//! The GENET FDT binding is reported on its own `genet:` line
//! (`kernel_core::genet_fdt::boot_report`), not here: ADR-0072/0073 keep
//! `/soc` out of this inventory.

use core::fmt::Write;

use kernel_core::fdt;
use kernel_core::hwdesc::{self, CompiledClaims, HwDescription, TableKind, Verdict};

use crate::arch::bootinfo;
use crate::bsp::board;
use crate::drivers::pl011::Pl011;

/// Print the `discover:` report. Call after the DTB is mapped RO (or after
/// it is known absent). Fail-open: parse errors become `unknown (…)` lines.
///
/// `dtb_mapped` is true only when `mmu::map` of the blob succeeded — a
/// surveyed pointer without a map must not be dereferenced.
///
/// `smp_seen` is the number of cores the kernel has observed alive (primary
/// + unparked secondaries), not the firmware spin-table entry count.
pub fn report(uart: &mut Pl011, dtb_mapped: bool, smp_seen: u64) {
    let claims = CompiledClaims {
        identity_ram_end: board::memmap::IDENTITY_RAM_END as u64,
        expected_cpus: board::memmap::EXPECTED_CPUS,
        model_prefix: board::memmap::EXPECTED_MODEL_PREFIX,
    };
    let identity_mib = claims.identity_ram_end / (1024 * 1024);

    if !dtb_mapped {
        let _ = writeln!(uart, "discover: model unknown (no dtb)");
        let _ = writeln!(uart, "discover: memory unknown (no dtb)");
        let _ = writeln!(uart, "discover: cpus unknown (no dtb)");
    } else {
        // SAFETY: caller mapped the blob RO; length from validated header.
        match unsafe { bootinfo::device_tree_slice() } {
            None => {
                let _ = writeln!(uart, "discover: model unknown (no dtb)");
                let _ = writeln!(uart, "discover: memory unknown (no dtb)");
                let _ = writeln!(uart, "discover: cpus unknown (no dtb)");
            }
            Some(bytes) => match fdt::extract(bytes) {
                Ok(x) => {
                    let hw = HwDescription::from_fdt(&x);
                    let r = hwdesc::reconcile(&claims, &hw);
                    print_model(uart, &hw, r.model);
                    print_memory(uart, &hw, r.memory, identity_mib);
                    print_cpus(uart, &hw, r.cpus, smp_seen);
                }
                Err(e) => {
                    let why = fdt_err(e);
                    let _ = writeln!(uart, "discover: model unknown ({why})");
                    let _ = writeln!(uart, "discover: memory unknown ({why})");
                    let _ = writeln!(uart, "discover: cpus unknown ({why})");
                }
            },
        }
    }

    // `off` for every image since ADR-0094 retired the panel, and written as a
    // constant rather than as a `cfg!` on a feature that no longer exists.
    //
    // The line stays: discovery reports what the image *claims* to carry, and
    // "no display compiled in" is a true claim about every image this tree
    // builds. It is also where a future panel announces itself — deleting it
    // would delete the slot along with the driver.
    let _ = writeln!(uart, "discover: display compiled=off (claim, not probed)");
}

fn fdt_err(e: fdt::FdtError) -> &'static str {
    match e {
        fdt::FdtError::Truncated => "truncated",
        fdt::FdtError::BadMagic => "bad magic",
        fdt::FdtError::OldVersion => "old version",
        fdt::FdtError::BadStructure => "bad structure",
        fdt::FdtError::TooDeep => "too deep",
        fdt::FdtError::BadCells => "bad cells",
    }
}

fn print_model(uart: &mut Pl011, hw: &HwDescription, v: Verdict) {
    match v {
        Verdict::Unknown => {
            let _ = writeln!(uart, "discover: model unknown (empty model)");
        }
        _ => {
            let model = hw.model.as_str();
            match hw.revision {
                Some(rev) => {
                    let _ = writeln!(uart, "discover: model \"{model}\" rev={rev:#x} (fdt)");
                }
                None => {
                    let _ = writeln!(uart, "discover: model \"{model}\" (fdt)");
                }
            }
        }
    }
}

fn print_memory(uart: &mut Pl011, hw: &HwDescription, v: Verdict, identity_mib: u64) {
    match v {
        Verdict::Unknown => {
            let _ = writeln!(uart, "discover: memory unknown (zero size memory)");
        }
        _ => {
            let mib = hw.memory_total / (1024 * 1024);
            let n = hw.memory_ranges;
            let ranges = if n == 1 { "range" } else { "ranges" };
            let verdict = match v {
                Verdict::Matches => "matches",
                Verdict::BeyondCompiledMap => "beyond compiled map",
                Verdict::Short => "short",
                Verdict::Differs => "differs",
                Verdict::Unknown => "unknown",
            };
            let _ = writeln!(
                uart,
                "discover: memory {mib} MiB ({n} {ranges}) {verdict} (identity {identity_mib} MiB)"
            );
        }
    }
}

fn print_cpus(uart: &mut Pl011, hw: &HwDescription, v: Verdict, smp_seen: u64) {
    match v {
        Verdict::Unknown => {
            let _ = writeln!(uart, "discover: cpus unknown (no cpu nodes)");
        }
        _ => {
            let verdict = match v {
                Verdict::Matches => "matches",
                Verdict::Differs => "differs",
                Verdict::BeyondCompiledMap | Verdict::Short | Verdict::Unknown => "differs",
            };
            let src = match hw.source {
                TableKind::Fdt => "fdt",
                TableKind::Cpuid => "cpuid",
                TableKind::PvhMemMap => "pvh",
            };
            let _ = writeln!(
                uart,
                "discover: cpus {} ({src}) smp-seen={smp_seen} {verdict}",
                hw.cpus
            );
        }
    }
}
