//! Device-tree binding for the BCM2711 GENET v5 backend.
//!
//! This is deliberately separate from the closed platform-information
//! extraction in [`crate::fdt`].  It translates only the GENET node's parent
//! bus, resolves its MDIO PHY, and returns a bounded binding contract.  No
//! address, interrupt, or PHY number is compiled into the backend.
//!
//! [`boot_report`] is the printable product line: FDT extract only, never an
//! MMIO probe, never a service bind. An unmapped blob is reported as absent
//! even if a slice is supplied.

use core::fmt::{self, Display, Formatter};

use crate::genet::{DmaWindow, DmaWindows};

const BEGIN_NODE: u32 = 1;
const END_NODE: u32 = 2;
const PROP: u32 = 3;
const NOP: u32 = 4;
const END: u32 = 9;
const MAX_DEPTH: usize = 8;
const MAX_RANGES: usize = 4;
const MAX_RANGE_BYTES: usize = 256;
const GENET_COMPAT: &[u8] = b"brcm,bcm2711-genet-v5";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadMagic,
    BadStructure,
    TooDeep,
    UnsupportedCells,
    Missing,
    Ambiguous,
    Incompatible,
    Disabled,
    InvalidRange,
    InvalidInterrupts,
    InvalidPhy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interrupt {
    /// First interrupt-specifier cell; the phandle is [`Binding::interrupt_parent`].
    pub specifier0: u32,
    pub number: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub mmio_base: u64,
    pub mmio_len: u64,
    pub interrupt_parent: u32,
    pub interrupts: [Interrupt; 2],
    pub dma: DmaWindows,
    pub phy_addr: u32,
    pub phy_mode_rgmii_rxid: bool,
}

/// Boot line for the GENET FDT binding. Not a device probe and not a
/// `discover:` fact (ADR-0072's closed inventory still excludes `/soc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    Binding(Binding),
    Unavailable(Unavailable),
}

/// Why the GENET binding is not printable as present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    NoDtb,
    Extract(Error),
}

/// Classify the boot-time GENET FDT report.
///
/// `dtb_mapped` is the MMU fact from the caller. A false flag is `NoDtb`
/// even when `bytes` is `Some`: an unmapped firmware blob must not be read.
pub fn boot_report(dtb_mapped: bool, bytes: Option<&[u8]>) -> Report {
    if !dtb_mapped {
        return Report::Unavailable(Unavailable::NoDtb);
    }
    match bytes {
        None => Report::Unavailable(Unavailable::NoDtb),
        Some(data) => match extract(data) {
            Ok(binding) => Report::Binding(binding),
            Err(error) => Report::Unavailable(Unavailable::Extract(error)),
        },
    }
}

impl Display for Report {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Report::Binding(binding) => {
                let phy = if binding.phy_mode_rgmii_rxid {
                    "rgmii-rxid"
                } else {
                    "other"
                };
                write!(
                    f,
                    "genet: binding ok base={:#x} len={:#x} phy={phy} (fdt, not probed)",
                    binding.mmio_base, binding.mmio_len
                )
            }
            Report::Unavailable(Unavailable::NoDtb) => f.write_str("genet: unavailable (no dtb)"),
            Report::Unavailable(Unavailable::Extract(error)) => {
                write!(f, "genet: unavailable ({error:?})")
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Range {
    child: u64,
    parent: u64,
    size: u64,
}

#[derive(Clone, Copy)]
struct State {
    depth: usize,
    scb: bool,
    genet: bool,
    mdio: bool,
    phy: bool,
    genet_count: u32,
    genet_reg: Option<(u64, u64)>,
    interrupts: [Interrupt; 2],
    interrupt_count: usize,
    status_okay: bool,
    compat: bool,
    phy_handle: Option<u32>,
    phy_candidate: Option<(u32, u32)>,
    phy_mode: bool,
    interrupt_parent: Option<u32>,
    scb_ac: u32,
    scb_sc: u32,
    ranges: [Range; MAX_RANGES],
    range_count: usize,
    dma_ranges: [Range; MAX_RANGES],
    dma_range_count: usize,
    ranges_raw: [u8; MAX_RANGE_BYTES],
    ranges_raw_len: usize,
    dma_ranges_raw: [u8; MAX_RANGE_BYTES],
    dma_ranges_raw_len: usize,
}

impl State {
    const fn new() -> Self {
        Self {
            depth: 0,
            scb: false,
            genet: false,
            mdio: false,
            phy: false,
            genet_count: 0,
            genet_reg: None,
            interrupts: [Interrupt {
                specifier0: 0,
                number: 0,
                flags: 0,
            }; 2],
            interrupt_count: 0,
            status_okay: false,
            compat: false,
            phy_handle: None,
            phy_candidate: None,
            phy_mode: false,
            interrupt_parent: None,
            scb_ac: 0,
            scb_sc: 0,
            ranges: [Range {
                child: 0,
                parent: 0,
                size: 0,
            }; MAX_RANGES],
            range_count: 0,
            dma_ranges: [Range {
                child: 0,
                parent: 0,
                size: 0,
            }; MAX_RANGES],
            dma_range_count: 0,
            ranges_raw: [0; MAX_RANGE_BYTES],
            ranges_raw_len: 0,
            dma_ranges_raw: [0; MAX_RANGE_BYTES],
            dma_ranges_raw_len: 0,
        }
    }
}

fn word(data: &[u8], off: usize) -> Result<u32, Error> {
    let b = data.get(off..off + 4).ok_or(Error::Truncated)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn string(data: &[u8], off: usize, end: usize) -> Result<&[u8], Error> {
    let b = data.get(off..end).ok_or(Error::BadStructure)?;
    let n = b.iter().position(|x| *x == 0).ok_or(Error::BadStructure)?;
    Ok(&b[..n])
}

fn cells(data: &[u8], n: usize) -> Result<u64, Error> {
    if !(1..=2).contains(&n) || data.len() != n * 4 {
        return Err(Error::UnsupportedCells);
    }
    let mut v = 0;
    for chunk in data.chunks_exact(4) {
        v = (v << 32) | u64::from(u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(v)
}

fn prop_u32(value: &[u8]) -> Result<u32, Error> {
    if value.len() != 4 {
        return Err(Error::BadStructure);
    }
    Ok(u32::from_be_bytes([value[0], value[1], value[2], value[3]]))
}

fn compatible(value: &[u8]) -> bool {
    value.split(|x| *x == 0).any(|x| x == GENET_COMPAT)
}

fn ranges(
    value: &[u8],
    child_ac: u32,
    parent_ac: u32,
    sc: u32,
    out: &mut [Range; MAX_RANGES],
    count: &mut usize,
) -> Result<(), Error> {
    let stride = (child_ac + parent_ac + sc) as usize * 4;
    if !(1..=2).contains(&child_ac)
        || !(1..=2).contains(&parent_ac)
        || !(1..=2).contains(&sc)
        || stride == 0
        || !value.len().is_multiple_of(stride)
    {
        return Err(Error::UnsupportedCells);
    }
    for chunk in value.chunks_exact(stride) {
        if *count == MAX_RANGES {
            return Err(Error::Ambiguous);
        }
        let child_bytes = child_ac as usize * 4;
        let parent_bytes = parent_ac as usize * 4;
        let child = cells(&chunk[..child_bytes], child_ac as usize)?;
        let parent_start = child_bytes;
        let parent = cells(
            &chunk[parent_start..parent_start + parent_bytes],
            parent_ac as usize,
        )?;
        let size = cells(&chunk[parent_start + parent_bytes..], sc as usize)?;
        if size == 0 || child.checked_add(size).is_none() || parent.checked_add(size).is_none() {
            return Err(Error::InvalidRange);
        }
        out[*count] = Range {
            child,
            parent,
            size,
        };
        *count += 1;
    }
    Ok(())
}

fn translate(
    address: u64,
    len: u64,
    map: &[Range; MAX_RANGES],
    count: usize,
) -> Result<u64, Error> {
    let end = address.checked_add(len).ok_or(Error::InvalidRange)?;
    for r in map.iter().take(count) {
        if address >= r.child && end <= r.child + r.size {
            // The Devicetree binding defines ranges as an ordered mapping;
            // the first applicable window wins.  BCM2711 deliberately ends
            // with a broad fallback range that overlaps its device windows.
            return r
                .parent
                .checked_add(address - r.child)
                .ok_or(Error::InvalidRange);
        }
    }
    Err(Error::InvalidRange)
}

/// Extract and validate the Pi 4 GENET v5 binding from a complete DTB.
pub fn extract(data: &[u8]) -> Result<Binding, Error> {
    if data.len() < 40 || word(data, 0)? != crate::fdt::FDT_MAGIC {
        return if data.len() < 40 {
            Err(Error::Truncated)
        } else {
            Err(Error::BadMagic)
        };
    }
    let total = word(data, 4)? as usize;
    let struct_off = word(data, 8)? as usize;
    let strings_off = word(data, 12)? as usize;
    let strings_len = word(data, 32)? as usize;
    let struct_len = word(data, 36)? as usize;
    let struct_end = struct_off.checked_add(struct_len).ok_or(Error::Truncated)?;
    let strings_end = strings_off
        .checked_add(strings_len)
        .ok_or(Error::Truncated)?;
    if total > data.len() || struct_end > total || strings_end > total {
        return Err(Error::Truncated);
    }
    let mut s = State::new();
    let mut off = struct_off;
    let mut budget = struct_len / 4 + 1;
    loop {
        budget = budget.checked_sub(1).ok_or(Error::BadStructure)?;
        let token = word(data, off)?;
        off += 4;
        match token {
            BEGIN_NODE => {
                let name = string(data, off, struct_end)?;
                off = (off + name.len() + 1).next_multiple_of(4);
                if s.depth == MAX_DEPTH {
                    return Err(Error::TooDeep);
                }
                let bare = name.split(|x| *x == b'@').next().unwrap_or(name);
                s.depth += 1;
                let parent = s.depth == 2 && bare == b"scb";
                let genet = s.depth == 3 && s.scb && bare == b"ethernet";
                let mdio = s.depth == 4 && s.genet && bare == b"mdio";
                let phy = s.depth == 5 && s.mdio && bare == b"ethernet-phy";
                if parent {
                    s.scb = true;
                } else if genet {
                    s.genet = true;
                    s.genet_count += 1;
                } else if mdio {
                    s.mdio = true;
                } else if phy {
                    s.phy = true;
                }
            }
            END_NODE => {
                if s.depth == 0 {
                    return Err(Error::BadStructure);
                }
                if s.depth == 5 {
                    s.phy = false;
                } else if s.depth == 4 {
                    s.mdio = false;
                } else if s.depth == 3 {
                    s.genet = false;
                } else if s.depth == 2 {
                    s.scb = false;
                }
                s.depth -= 1;
            }
            PROP => {
                let len = word(data, off)? as usize;
                let name_off = word(data, off + 4)? as usize;
                let value_off = off + 8;
                let value_end = value_off.checked_add(len).ok_or(Error::BadStructure)?;
                if value_end > struct_end {
                    return Err(Error::BadStructure);
                }
                let name = string(
                    data,
                    strings_off
                        .checked_add(name_off)
                        .ok_or(Error::BadStructure)?,
                    strings_end,
                )?;
                let value = &data[value_off..value_end];
                absorb(&mut s, name, value)?;
                off = value_end.next_multiple_of(4);
            }
            NOP => {}
            END => {
                if s.depth != 0 {
                    return Err(Error::BadStructure);
                }
                break;
            }
            _ => return Err(Error::BadStructure),
        }
    }
    if s.genet_count != 1 {
        return Err(if s.genet_count == 0 {
            Error::Missing
        } else {
            Error::Ambiguous
        });
    }
    if !s.compat {
        return Err(Error::Incompatible);
    }
    if !s.status_okay {
        return Err(Error::Disabled);
    }
    if !s.phy_mode {
        return Err(Error::Incompatible);
    }
    if s.scb_ac == 0 || s.scb_sc == 0 || s.ranges_raw_len == 0 || s.dma_ranges_raw_len == 0 {
        return Err(Error::Missing);
    }
    ranges(
        &s.ranges_raw[..s.ranges_raw_len],
        s.scb_ac,
        2,
        s.scb_sc,
        &mut s.ranges,
        &mut s.range_count,
    )?;
    ranges(
        &s.dma_ranges_raw[..s.dma_ranges_raw_len],
        s.scb_ac,
        2,
        s.scb_sc,
        &mut s.dma_ranges,
        &mut s.dma_range_count,
    )?;
    let (bus_addr, mmio_len) = s.genet_reg.ok_or(Error::Missing)?;
    if mmio_len != 0x10000 {
        return Err(Error::Incompatible);
    }
    let mmio_base = translate(bus_addr, mmio_len, &s.ranges, s.range_count)?;
    let mut dma_windows = [DmaWindow {
        base: 0,
        cpu_base: 0,
        len: 0,
    }; MAX_RANGES];
    for (index, range) in s.dma_ranges.iter().take(s.dma_range_count).enumerate() {
        dma_windows[index] =
            DmaWindow::mapped(range.child, range.parent, range.size).ok_or(Error::InvalidRange)?;
    }
    let dma = DmaWindows::new(dma_windows, s.dma_range_count as u8).ok_or(Error::InvalidRange)?;
    let phy_addr = s
        .phy_candidate
        .and_then(|(phandle, addr)| (Some(phandle) == s.phy_handle).then_some(addr))
        .ok_or(Error::InvalidPhy)?;
    if s.interrupt_count != 2 || s.interrupts[0].flags != 4 || s.interrupts[1].flags != 4 {
        return Err(Error::InvalidInterrupts);
    }
    let interrupt_parent = s.interrupt_parent.ok_or(Error::InvalidInterrupts)?;
    Ok(Binding {
        mmio_base,
        mmio_len,
        interrupt_parent,
        interrupts: s.interrupts,
        dma,
        phy_addr,
        phy_mode_rgmii_rxid: s.phy_mode,
    })
}

fn absorb(s: &mut State, name: &[u8], value: &[u8]) -> Result<(), Error> {
    if s.depth == 1 && name == b"interrupt-parent" {
        s.interrupt_parent = Some(prop_u32(value)?);
    } else if s.depth == 2 && s.scb {
        match name {
            b"#address-cells" => s.scb_ac = prop_u32(value)?,
            b"#size-cells" => s.scb_sc = prop_u32(value)?,
            b"ranges" => {
                if value.len() > MAX_RANGE_BYTES {
                    return Err(Error::Ambiguous);
                }
                s.ranges_raw[..value.len()].copy_from_slice(value);
                s.ranges_raw_len = value.len();
            }
            b"dma-ranges" => {
                if value.len() > MAX_RANGE_BYTES {
                    return Err(Error::Ambiguous);
                }
                s.dma_ranges_raw[..value.len()].copy_from_slice(value);
                s.dma_ranges_raw_len = value.len();
            }
            _ => {}
        }
    } else if s.depth == 3 && s.genet {
        match name {
            b"compatible" => s.compat = compatible(value),
            b"status" => s.status_okay = value.strip_suffix(&[0]).unwrap_or(value) == b"okay",
            b"reg" => {
                if value.len() != 16 {
                    return Err(Error::BadStructure);
                }
                s.genet_reg = Some((cells(&value[..8], 2)?, cells(&value[8..], 2)?));
            }
            b"interrupts" => {
                if value.len() != 24 {
                    return Err(Error::InvalidInterrupts);
                }
                for (i, chunk) in value.chunks_exact(12).enumerate() {
                    s.interrupts[i] = Interrupt {
                        specifier0: prop_u32(&chunk[..4])?,
                        number: prop_u32(&chunk[4..8])?,
                        flags: prop_u32(&chunk[8..])?,
                    };
                }
                s.interrupt_count = 2;
            }
            b"phy-handle" => s.phy_handle = Some(prop_u32(value)?),
            b"phy-mode" => s.phy_mode = value.strip_suffix(&[0]).unwrap_or(value) == b"rgmii-rxid",
            _ => {}
        }
    } else if s.depth == 5 && s.phy {
        if name == b"phandle" {
            let p = prop_u32(value)?;
            let addr = s.phy_candidate.map(|x| x.1).unwrap_or(0);
            s.phy_candidate = Some((p, addr));
        }
        if name == b"reg" {
            let addr = prop_u32(value)?;
            let p = s.phy_candidate.map(|x| x.0).unwrap_or(0);
            s.phy_candidate = Some((p, addr));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const PI4: &[u8] = include_bytes!("../tests/fixtures/bcm2711-rpi-4-b.dtb");

    #[test]
    fn extracts_pi4_binding() {
        let x = extract(PI4).unwrap();
        assert_eq!(x.mmio_base, 0xfd580000);
        assert_eq!(x.mmio_len, 0x10000);
        assert_eq!(x.interrupt_parent, 1);
        assert_eq!(x.interrupts[0].number, 0x9d);
        assert_eq!(x.interrupts[1].number, 0x9e);
        assert_eq!(x.phy_addr, 1);
        assert!(x.phy_mode_rgmii_rxid);
        assert_eq!(x.dma.count, 2);
        assert!(x.dma.contains(0x47c000000, 0x1000));
        // Frame-pool CPU addresses sit in the first parent window.
        assert!(x.dma.map_cpu(0x411d000, 1536).is_ok());
    }

    #[test]
    fn refuses_missing_genet() {
        let mut bad = PI4.to_vec();
        let needle = b"brcm,bcm2711-genet-v5";
        let pos = bad.windows(needle.len()).position(|x| x == needle).unwrap();
        bad[pos] = b'x';
        assert_eq!(extract(&bad), Err(Error::Incompatible));
    }

    #[test]
    fn boot_report_reads_mapped_fixture() {
        let report = boot_report(true, Some(PI4));
        assert!(matches!(report, Report::Binding(_)));
        assert_eq!(
            report.to_string(),
            "genet: binding ok base=0xfd580000 len=0x10000 phy=rgmii-rxid (fdt, not probed)"
        );
    }

    #[test]
    fn boot_report_ignores_bytes_when_unmapped() {
        let report = boot_report(false, Some(PI4));
        assert_eq!(report, Report::Unavailable(Unavailable::NoDtb));
        assert_eq!(report.to_string(), "genet: unavailable (no dtb)");
    }

    #[test]
    fn boot_report_mapped_without_slice_is_absent() {
        let report = boot_report(true, None);
        assert_eq!(report, Report::Unavailable(Unavailable::NoDtb));
    }

    #[test]
    fn boot_report_extract_refusal_is_unavailable() {
        let report = boot_report(true, Some(&[]));
        assert_eq!(
            report,
            Report::Unavailable(Unavailable::Extract(Error::Truncated))
        );
        assert_eq!(report.to_string(), "genet: unavailable (Truncated)");
    }

    #[test]
    fn boot_report_missing_node_matches_qemu_guest_dtb() {
        // QEMU raspi4b deletes `/scb/ethernet` after loading the fixture.
        // The guest line is this Display, not `binding ok`.
        assert_eq!(
            Report::Unavailable(Unavailable::Extract(Error::Missing)).to_string(),
            "genet: unavailable (Missing)"
        );
    }
}
