//! Kernel memory layout: turning linker boundaries into mapped regions.
//!
//! The kernel reads the addresses from linker symbols; everything after that —
//! which region gets which permissions, and whether the result is coherent —
//! is arithmetic, and lives here so it can be tested without a board.
//!
//! The builder *validates* rather than trusting: regions must be ascending,
//! non-overlapping and page-aligned, and nothing may be writable and
//! executable at once. A linker script edit that silently overlaps two regions
//! would otherwise produce a map that boots and protects nothing.

use crate::paging::{MemKind, PAGE_SIZE, Perms};

/// A region to identity-map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub base: u64,
    pub len: u64,
    pub kind: MemKind,
    pub perms: Perms,
    /// Shown in diagnostics when mapping fails.
    pub name: &'static str,
}

impl Region {
    /// One past the last byte.
    pub const fn end(&self) -> u64 {
        self.base + self.len
    }

    /// True if this region can be written and executed — the thing W^X exists
    /// to prevent.
    pub const fn is_write_execute(&self) -> bool {
        self.perms.write && self.perms.execute
    }
}

/// A device MMIO window supplied by the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceWindow {
    pub base: u64,
    pub len: u64,
    pub name: &'static str,
}

/// Linker-provided boundaries. Every address must be page aligned.
///
/// Pairs are `(start, end)`, end exclusive.
#[derive(Clone, Copy, Debug)]
pub struct Boundaries {
    pub image_start: u64,
    pub text: (u64, u64),
    pub rodata: (u64, u64),
    pub data: (u64, u64),
    pub pagetables: (u64, u64),
    /// Deliberately *not* mapped; present so the builder can check that the
    /// stack really is fenced off from what precedes it.
    pub guard: (u64, u64),
    pub stack: (u64, u64),
    pub heap: (u64, u64),
}

/// Why a layout could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// The output slice is shorter than the number of regions.
    TooManyRegions,
    /// A boundary is not page aligned.
    Unaligned { name: &'static str, addr: u64 },
    /// A region ends at or before it starts.
    Empty { name: &'static str },
    /// Two regions overlap, or they are not in ascending order.
    Overlap {
        first: &'static str,
        second: &'static str,
    },
    /// A region is both writable and executable.
    WriteExecute { name: &'static str },
    /// The guard page is mapped by some region, so it guards nothing.
    GuardMapped { by: &'static str },
    /// The guard page is empty, or does not sit immediately below the stack.
    ///
    /// A zero-length guard passes every "is it covered?" test vacuously, which
    /// is exactly the shape of deleting it from the linker script.
    GuardIneffective {
        guard_start: u64,
        guard_end: u64,
        stack_base: u64,
    },
}

const fn region(
    (base, end): (u64, u64),
    kind: MemKind,
    perms: Perms,
    name: &'static str,
) -> Region {
    Region {
        base,
        len: end.saturating_sub(base),
        kind,
        perms,
        name,
    }
}

/// Build the kernel's region list into `out`, validated.
///
/// Returns the filled prefix. RAM below the image is mapped because the
/// firmware's mailboxes and the secondary-core spin table live there.
pub fn kernel_regions<'a>(
    bounds: &Boundaries,
    devices: &[DeviceWindow],
    out: &'a mut [Region],
) -> Result<&'a mut [Region], LayoutError> {
    let ram_count = 7;
    if out.len() < ram_count + devices.len() {
        return Err(LayoutError::TooManyRegions);
    }

    out[0] = region(
        (0, bounds.image_start),
        MemKind::NormalWb,
        Perms::RW,
        "low RAM",
    );
    out[1] = region(bounds.text, MemKind::NormalWb, Perms::RX, ".text");
    out[2] = region(bounds.rodata, MemKind::NormalWb, Perms::RO, ".rodata");
    out[3] = region(bounds.data, MemKind::NormalWb, Perms::RW, ".data/.bss");
    out[4] = region(
        bounds.pagetables,
        MemKind::NormalWb,
        Perms::RW,
        "page tables",
    );
    // The guard page sits between the arena and the stack, and is skipped.
    out[5] = region(bounds.stack, MemKind::NormalWb, Perms::RW, "stack");
    out[6] = region(bounds.heap, MemKind::NormalWb, Perms::RW, "heap");

    for (slot, window) in out[ram_count..].iter_mut().zip(devices) {
        *slot = Region {
            base: window.base,
            len: window.len,
            kind: MemKind::Device,
            perms: Perms::RW,
            name: window.name,
        };
    }

    let filled = &mut out[..ram_count + devices.len()];
    validate(filled, bounds)?;
    Ok(filled)
}

/// Check the invariants the map depends on but the type system cannot state.
fn validate(regions: &[Region], bounds: &Boundaries) -> Result<(), LayoutError> {
    // Check the guard exists before checking that nothing covers it: with an
    // empty range the second test is vacuously true, so a deleted guard page
    // would sail through. It must be at least one page and end exactly where
    // the stack begins, or an overflow lands somewhere real.
    if bounds.guard.1 < bounds.guard.0 + PAGE_SIZE || bounds.guard.1 != bounds.stack.0 {
        return Err(LayoutError::GuardIneffective {
            guard_start: bounds.guard.0,
            guard_end: bounds.guard.1,
            stack_base: bounds.stack.0,
        });
    }

    for r in regions.iter() {
        if r.len == 0 {
            return Err(LayoutError::Empty { name: r.name });
        }
        if r.base % PAGE_SIZE != 0 || r.len % PAGE_SIZE != 0 {
            return Err(LayoutError::Unaligned {
                name: r.name,
                addr: r.base,
            });
        }
        if r.is_write_execute() {
            return Err(LayoutError::WriteExecute { name: r.name });
        }
        // The guard page must not fall inside anything.
        if r.base < bounds.guard.1 && bounds.guard.0 < r.end() {
            return Err(LayoutError::GuardMapped { by: r.name });
        }
    }

    // Ascending and disjoint. Device windows sit far above RAM, so checking
    // consecutive pairs is enough — and if that ever stops holding, this fires.
    for pair in regions.windows(2) {
        if pair[1].base < pair[0].end() {
            return Err(LayoutError::Overlap {
                first: pair[0].name,
                second: pair[1].name,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Boundaries with the shape `link.ld` produces, at round numbers.
    fn bounds() -> Boundaries {
        Boundaries {
            image_start: 0x8_0000,
            text: (0x8_0000, 0x8_6000),
            rodata: (0x8_6000, 0x8_7000),
            data: (0x8_7000, 0x8_8000),
            pagetables: (0x8_8000, 0x9_8000),
            guard: (0x9_8000, 0x9_9000),
            stack: (0x9_9000, 0xA_9000),
            heap: (0xA_9000, 0x40A_9000),
        }
    }

    fn devices() -> [DeviceWindow; 2] {
        [
            DeviceWindow {
                base: 0xFE00_0000,
                len: 0x0100_0000,
                name: "peripherals",
            },
            DeviceWindow {
                base: 0xFF84_0000,
                len: 0x4000,
                name: "GIC",
            },
        ]
    }

    fn build(b: &Boundaries) -> Result<[Region; 9], LayoutError> {
        let mut out = [Region {
            base: 0,
            len: 0,
            kind: MemKind::NormalWb,
            perms: Perms::RW,
            name: "unused",
        }; 9];
        kernel_regions(b, &devices(), &mut out)?;
        Ok(out)
    }

    /// The exact addresses the linker produces on the board, as reported by a
    /// boot-time probe. Guards against reasoning about the validator instead of
    /// running it.
    #[test]
    fn the_real_board_layout_is_accepted() {
        let b = Boundaries {
            image_start: 0x8_0000,
            text: (0x8_0000, 0x8_6000),
            rodata: (0x8_6000, 0x8_7000),
            data: (0x8_7000, 0x8_C000),
            pagetables: (0x8_C000, 0x9_C000),
            guard: (0x9_C000, 0x9_D000),
            stack: (0x9_D000, 0xA_D000),
            heap: (0xA_D000, 0x40A_D000),
        };
        assert_eq!(b.guard.1 - b.guard.0, 0x1000, "one page");
        assert_eq!(b.guard.1, b.stack.0, "guard touches the stack");
        build(&b).expect("the real layout must validate");
    }

    #[test]
    fn the_expected_layout_builds() {
        let regions = build(&bounds()).unwrap();
        assert_eq!(regions.len(), 9);
        assert_eq!(regions[1].name, ".text");
        assert_eq!(regions[1].perms, Perms::RX);
    }

    /// The invariant W^X exists for. A permission table edit that made any
    /// region writable *and* executable would otherwise map fine and protect
    /// nothing.
    #[test]
    fn nothing_is_both_writable_and_executable() {
        for r in build(&bounds()).unwrap() {
            assert!(!r.is_write_execute(), "{} is W+X", r.name);
        }
    }

    #[test]
    fn regions_are_ascending_and_disjoint() {
        let regions = build(&bounds()).unwrap();
        for pair in regions.windows(2) {
            assert!(
                pair[0].end() <= pair[1].base,
                "{} overlaps {}",
                pair[0].name,
                pair[1].name
            );
        }
    }

    /// The guard page is the whole point of the stack fence: if any region
    /// covers it, an overflow writes real memory instead of faulting.
    #[test]
    fn the_guard_page_is_covered_by_nothing() {
        let b = bounds();
        for r in build(&b).unwrap() {
            assert!(
                r.end() <= b.guard.0 || r.base >= b.guard.1,
                "{} maps the guard page",
                r.name
            );
        }
    }

    /// A pure overlap, away from the guard page: `validate` checks the guard
    /// first, so a mutation that does both would report `GuardMapped` and this
    /// test would not exercise the ordering check at all.
    #[test]
    fn an_overlap_is_rejected() {
        let mut b = bounds();
        // `.rodata` runs a page into `.data`.
        b.rodata.1 = b.data.0 + 0x1000;
        assert!(matches!(
            build(&b),
            Err(LayoutError::Overlap {
                first: ".rodata",
                second: ".data/.bss"
            })
        ));
    }

    /// Deleting the guard page is the failure this must catch, and a naive
    /// "is it covered?" check passes it vacuously — an empty range is inside
    /// nothing. Found by setting `GUARD_PAGE_SIZE = 0` in `link.ld` and
    /// watching the kernel map it happily.
    #[test]
    fn a_zero_length_guard_page_is_rejected() {
        let mut b = bounds();
        b.guard.1 = b.guard.0;
        b.stack.0 = b.guard.0;
        assert!(matches!(
            build(&b),
            Err(LayoutError::GuardIneffective { .. })
        ));
    }

    /// A guard that does not touch the stack leaves a gap the stack can grow
    /// past without ever hitting it.
    #[test]
    fn a_guard_page_detached_from_the_stack_is_rejected() {
        let mut b = bounds();
        b.guard.1 -= 0x1000;
        assert!(matches!(
            build(&b),
            Err(LayoutError::GuardIneffective { .. })
        ));
    }

    #[test]
    fn a_region_covering_the_guard_page_is_rejected() {
        let mut b = bounds();
        // The arena runs past its end, swallowing the guard.
        b.pagetables.1 = b.guard.1;
        assert!(matches!(build(&b), Err(LayoutError::GuardMapped { .. })));
    }

    #[test]
    fn a_misaligned_boundary_is_rejected() {
        let mut b = bounds();
        b.rodata.0 += 0x800;
        assert!(matches!(build(&b), Err(LayoutError::Unaligned { .. })));
    }

    #[test]
    fn an_empty_region_is_rejected() {
        let mut b = bounds();
        b.rodata.1 = b.rodata.0;
        assert!(matches!(build(&b), Err(LayoutError::Empty { .. })));
    }

    #[test]
    fn a_short_output_slice_is_rejected() {
        let mut out = [Region {
            base: 0,
            len: 0,
            kind: MemKind::NormalWb,
            perms: Perms::RW,
            name: "unused",
        }; 4];
        assert_eq!(
            kernel_regions(&bounds(), &devices(), &mut out),
            Err(LayoutError::TooManyRegions)
        );
    }

    #[test]
    fn device_windows_are_never_executable() {
        for r in build(&bounds()).unwrap() {
            if r.kind == MemKind::Device {
                assert!(!r.perms.execute, "{} is executable device memory", r.name);
            }
        }
    }
}
