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

/// A stack and the unmapped page immediately below it.
///
/// The two belong together: a guard page is only a guard if it sits directly
/// under the stack it protects and nothing maps it. Keeping them in one type
/// means a second stack cannot be added while forgetting either half.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardedStack {
    /// Deliberately *not* mapped; present so the builder can check that the
    /// stack really is fenced off from what precedes it.
    pub guard: (u64, u64),
    pub stack: (u64, u64),
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
    /// The stack the kernel runs on, via `SP_EL0`.
    pub kernel_stack: GuardedStack,
    /// The stack exceptions are taken on, via `SP_EL1`. Separate so that a
    /// kernel stack overflow can be *reported*: a handler that saved its trap
    /// frame below the overflow would fault again and hang instead.
    pub exception_stack: GuardedStack,
    pub heap: (u64, u64),
    /// Named phys frame pool for user AS tables/pages (ADR-0012), exclusive end.
    pub frame_pool: (u64, u64),
}

impl Boundaries {
    /// Both guarded stacks, so validation is written once.
    const fn stacks(&self) -> [&GuardedStack; 2] {
        [&self.kernel_stack, &self.exception_stack]
    }
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
    let ram_count = 9;
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
    // Each stack's guard page sits immediately below it, and is skipped.
    out[5] = region(
        bounds.exception_stack.stack,
        MemKind::NormalWb,
        Perms::RW,
        bounds.exception_stack.name,
    );
    out[6] = region(
        bounds.kernel_stack.stack,
        MemKind::NormalWb,
        Perms::RW,
        bounds.kernel_stack.name,
    );
    out[7] = region(bounds.heap, MemKind::NormalWb, Perms::RW, "heap");
    // ADR-0012: named user/AS frame pool, immediately after the heap window.
    out[8] = region(
        bounds.frame_pool,
        MemKind::NormalWb,
        Perms::RW,
        "frame pool",
    );

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

/// Check one stack+guard pair (bootstrap stacks or a heap-allocated task stack).
///
/// The guard must be at least one page and end exactly where the stack begins.
/// Coverage by a mapped region is checked separately by [`validate`] for the
/// kernel map, or by the caller after [`crate`]-level unmap for task stacks.
pub const fn validate_guarded_stack(guarded: &GuardedStack) -> Result<(), LayoutError> {
    if guarded.guard.1 < guarded.guard.0 + PAGE_SIZE || guarded.guard.1 != guarded.stack.0 {
        return Err(LayoutError::GuardIneffective {
            guard_start: guarded.guard.0,
            guard_end: guarded.guard.1,
            stack_base: guarded.stack.0,
        });
    }
    if guarded.stack.1 <= guarded.stack.0 {
        return Err(LayoutError::Empty { name: guarded.name });
    }
    if !guarded.guard.0.is_multiple_of(PAGE_SIZE)
        || !guarded.stack.0.is_multiple_of(PAGE_SIZE)
        || !guarded.stack.1.is_multiple_of(PAGE_SIZE)
    {
        return Err(LayoutError::Unaligned {
            name: guarded.name,
            addr: guarded.guard.0,
        });
    }
    Ok(())
}

/// Check the invariants the map depends on but the type system cannot state.
fn validate(regions: &[Region], bounds: &Boundaries) -> Result<(), LayoutError> {
    // Check each guard exists before checking that nothing covers it: with an
    // empty range the second test is vacuously true, so a deleted guard page
    // would sail through. It must be at least one page and end exactly where
    // its stack begins, or an overflow lands somewhere real.
    for guarded in bounds.stacks() {
        validate_guarded_stack(guarded)?;
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
        // No guard page may fall inside anything.
        for guarded in bounds.stacks() {
            if r.base < guarded.guard.1 && guarded.guard.0 < r.end() {
                return Err(LayoutError::GuardMapped { by: r.name });
            }
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

/// The private VA window an EL0 agent runs in (ADR-0014).
///
/// Page 0 is the agent's text, mapped `USER_RX`; the pages above it are its
/// stack, mapped `USER_RW`, with `SP_EL0` starting at the top. Fixed by the BSP
/// rather than negotiated — there is no loader and no binary format yet, so the
/// layout *is* the ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserWindow {
    /// Lowest VA: the text page.
    pub base: u64,
    /// Total pages, text included. Must be at least 2 — one text, one stack.
    pub pages: usize,
    /// Page size in bytes.
    pub frame: usize,
}

/// Why an access to a [`UserWindow`] was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowError {
    /// Fewer than two pages: no room for both text and a stack.
    TooSmall,
    /// The write would leave the text page.
    OutOfTextPage,
}

impl UserWindow {
    /// Exclusive top of the window — the initial `SP_EL0`, since the stack
    /// grows down.
    #[inline]
    pub const fn stack_top(&self) -> u64 {
        self.base + (self.pages as u64) * (self.frame as u64)
    }

    /// Where EL0 starts executing: the beginning of the text page.
    #[inline]
    pub const fn entry(&self) -> u64 {
        self.base
    }

    /// True for the page that must be mapped executable rather than writable.
    ///
    /// Exactly one, and it is the lowest: W^X applies to a user window as much
    /// as to the kernel map, so no page is ever both.
    #[inline]
    pub const fn is_text_page(&self, index: usize) -> bool {
        index == 0
    }

    #[inline]
    pub const fn validate(&self) -> Result<(), WindowError> {
        if self.pages < 2 {
            return Err(WindowError::TooSmall);
        }
        Ok(())
    }

    /// Bound a write of `len` bytes at `offset` into the **text page**.
    ///
    /// One page, not the whole window, and the distinction is the point. The
    /// kernel pokes user text through the identity map using the physical
    /// address of page 0 — the pages above it come from separate frame
    /// allocations and are contiguous only by accident of the pool's free
    /// order. Validating against `pages * frame` licensed exactly the write it
    /// should refuse: at any offset past the first page it lands in whatever
    /// frame happens to follow, which after a create/destroy cycle is another
    /// address space's page tables.
    ///
    /// ```
    /// use kernel_core::layout::{UserWindow, WindowError};
    ///
    /// let window = UserWindow { base: 0x4000_0000, pages: 4, frame: 0x1000 };
    ///
    /// // The whole text page is fair game.
    /// assert_eq!(window.bound_text_write(0, 0x1000), Ok(()));
    ///
    /// // One byte past it is refused — even though the *window* is 16 KiB,
    /// // the write goes to page 0's physical address and nowhere else.
    /// assert_eq!(
    ///     window.bound_text_write(0x1000, 1),
    ///     Err(WindowError::OutOfTextPage)
    /// );
    /// ```
    #[inline]
    pub const fn bound_text_write(&self, offset: usize, len: usize) -> Result<(), WindowError> {
        match offset.checked_add(len) {
            Some(end) if end <= self.frame => Ok(()),
            _ => Err(WindowError::OutOfTextPage),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window `bsp::rpi4` fixes: 4 pages of 4 KiB at 0x4000_0000.
    const WIN: UserWindow = UserWindow {
        base: 0x4000_0000,
        pages: 4,
        frame: 0x1000,
    };

    #[test]
    fn the_window_starts_where_el0_enters_and_ends_where_its_stack_begins() {
        assert_eq!(WIN.entry(), 0x4000_0000);
        assert_eq!(WIN.stack_top(), 0x4000_4000, "SP_EL0 starts at the top");
        assert!(WIN.validate().is_ok());
    }

    #[test]
    fn exactly_one_page_is_text() {
        // W^X inside the user window too: the executable page is never one of
        // the writable ones.
        assert!(WIN.is_text_page(0));
        for i in 1..WIN.pages {
            assert!(!WIN.is_text_page(i), "page {i} is stack");
        }
    }

    #[test]
    fn a_window_with_no_room_for_a_stack_is_refused() {
        let cramped = UserWindow { pages: 1, ..WIN };
        assert_eq!(cramped.validate(), Err(WindowError::TooSmall));
    }

    #[test]
    fn a_text_write_may_fill_the_page_and_no_more() {
        assert_eq!(WIN.bound_text_write(0, 0x1000), Ok(()), "exactly one page");
        assert_eq!(
            WIN.bound_text_write(0, 0x1001),
            Err(WindowError::OutOfTextPage)
        );
        assert_eq!(WIN.bound_text_write(0xFFF, 1), Ok(()), "last byte");
        assert_eq!(
            WIN.bound_text_write(0xFFF, 2),
            Err(WindowError::OutOfTextPage)
        );
    }

    #[test]
    fn a_write_past_the_text_page_is_refused_even_though_the_window_is_bigger() {
        // The defect. The bound used to be `pages * frame`, which made every
        // offset in the window look legal — while the write went to the
        // physical address of page 0 alone, so anything past 0x1000 landed in
        // whatever frame followed it in the pool.
        assert_eq!(
            WIN.bound_text_write(0x1000, 1),
            Err(WindowError::OutOfTextPage),
            "still inside the window, already outside the page being written"
        );
        assert_eq!(
            WIN.bound_text_write(0x3000, 4),
            Err(WindowError::OutOfTextPage)
        );
    }

    #[test]
    fn an_offset_that_would_overflow_is_refused_not_wrapped() {
        // `offset + len` on a 64-bit host wraps to something small and looks
        // legal. The check is `checked_add` for that reason.
        assert_eq!(
            WIN.bound_text_write(usize::MAX, 1),
            Err(WindowError::OutOfTextPage)
        );
        assert_eq!(
            WIN.bound_text_write(usize::MAX - 1, 8),
            Err(WindowError::OutOfTextPage)
        );
    }

    #[test]
    fn an_empty_write_is_allowed_anywhere_inside_the_page() {
        assert_eq!(WIN.bound_text_write(0, 0), Ok(()));
        assert_eq!(WIN.bound_text_write(0x1000, 0), Ok(()), "the boundary");
    }

    /// Boundaries with the shape `link.ld` produces, at round numbers.
    fn bounds() -> Boundaries {
        Boundaries {
            image_start: 0x8_0000,
            text: (0x8_0000, 0x8_6000),
            rodata: (0x8_6000, 0x8_7000),
            data: (0x8_7000, 0x8_8000),
            pagetables: (0x8_8000, 0x9_8000),
            exception_stack: GuardedStack {
                guard: (0x9_8000, 0x9_9000),
                stack: (0x9_9000, 0x9_D000),
                name: "exception stack",
            },
            kernel_stack: GuardedStack {
                guard: (0x9_D000, 0x9_E000),
                stack: (0x9_E000, 0xA_E000),
                name: "stack",
            },
            heap: (0xA_E000, 0x40A_E000),
            // 2 MiB frame pool immediately after heap (ADR-0012 shape).
            frame_pool: (0x40A_E000, 0x40A_E000 + 0x20_0000),
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

    fn build(b: &Boundaries) -> Result<[Region; 11], LayoutError> {
        let mut out = [Region {
            base: 0,
            len: 0,
            kind: MemKind::NormalWb,
            perms: Perms::RW,
            name: "unused",
        }; 11];
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
            exception_stack: GuardedStack {
                guard: (0x9_C000, 0x9_D000),
                stack: (0x9_D000, 0xA_1000),
                name: "exception stack",
            },
            kernel_stack: GuardedStack {
                guard: (0xA_1000, 0xA_2000),
                stack: (0xA_2000, 0xB_2000),
                name: "stack",
            },
            heap: (0xB_2000, 0x40B_2000),
            frame_pool: (0x40B_2000, 0x40B_2000 + 0x20_0000),
        };
        for guarded in [&b.exception_stack, &b.kernel_stack] {
            assert_eq!(guarded.guard.1 - guarded.guard.0, 0x1000, "one page");
            assert_eq!(guarded.guard.1, guarded.stack.0, "guard touches its stack");
        }
        build(&b).expect("the real layout must validate");
    }

    #[test]
    fn the_expected_layout_builds() {
        let regions = build(&bounds()).unwrap();
        assert_eq!(regions.len(), 11);
        assert_eq!(regions[1].name, ".text");
        assert_eq!(regions[1].perms, Perms::RX);
        assert_eq!(regions[8].name, "frame pool");
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
    fn the_guard_pages_are_covered_by_nothing() {
        let b = bounds();
        for guarded in [&b.kernel_stack, &b.exception_stack] {
            for r in build(&b).unwrap() {
                assert!(
                    r.end() <= guarded.guard.0 || r.base >= guarded.guard.1,
                    "{} maps the {} guard page",
                    r.name,
                    guarded.name
                );
            }
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
        b.kernel_stack.guard.1 = b.kernel_stack.guard.0;
        b.kernel_stack.stack.0 = b.kernel_stack.guard.0;
        assert!(matches!(
            build(&b),
            Err(LayoutError::GuardIneffective { .. })
        ));
    }

    #[test]
    fn validate_guarded_stack_accepts_a_task_shaped_pair() {
        let g = GuardedStack {
            guard: (0x10_0000, 0x10_1000),
            stack: (0x10_1000, 0x10_5000),
            name: "task",
        };
        assert!(validate_guarded_stack(&g).is_ok());
    }

    #[test]
    fn validate_guarded_stack_rejects_a_detached_guard() {
        let g = GuardedStack {
            guard: (0x10_0000, 0x10_1000),
            stack: (0x10_2000, 0x10_5000),
            name: "task",
        };
        assert!(matches!(
            validate_guarded_stack(&g),
            Err(LayoutError::GuardIneffective { .. })
        ));
    }

    /// The same for the exception stack. Validation is written once over both,
    /// and this is what keeps that true: a check that only ever looked at the
    /// kernel stack would pass every other test in this file.
    #[test]
    fn a_zero_length_exception_guard_page_is_rejected() {
        let mut b = bounds();
        b.exception_stack.guard.1 = b.exception_stack.guard.0;
        b.exception_stack.stack.0 = b.exception_stack.guard.0;
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
        b.kernel_stack.guard.1 -= 0x1000;
        assert!(matches!(
            build(&b),
            Err(LayoutError::GuardIneffective { .. })
        ));
    }

    #[test]
    fn a_region_covering_the_guard_page_is_rejected() {
        let mut b = bounds();
        // The arena runs past its end, swallowing the guard.
        b.pagetables.1 = b.exception_stack.guard.1;
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
