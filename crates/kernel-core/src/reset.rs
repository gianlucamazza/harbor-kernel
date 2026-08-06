//! BCM2711 `PM_RSTS` decode — why the board last came up.
//!
//! The kernel's `halt()` is `loop { wfe }` with IRQs masked and cannot exit, so
//! a board that boots again after `*** halt ***` was reset by something
//! outside this kernel. On 2026-08-06 that happened twice during a hardware
//! session and nobody could say what did it — the only observed behaviour in
//! this project with no account of itself.
//!
//! Guessing produced three plausible stories (firmware watchdog, brownout,
//! a glitch on the supply) and no way to choose between them. The silicon
//! records the answer: `PM_RSTS` latches the cause of the last reset, and its
//! bits distinguish exactly those cases.
//!
//! Bit names follow Linux `drivers/watchdog/bcm2835_wdt.c`, which is the only
//! written-down source for this register — Broadcom's public datasheet
//! documents the PM block as "not for publication". The three flavours of each
//! cause (`…RQ`, `…RF`, `…RH`) are the request, the "full" reset and the
//! "hard" reset the block distinguishes; for the question this answers, any of
//! them means the same thing, which is why [`ResetCause`] collapses them.

/// Why the board last reset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetCause {
    /// Power was applied. The expected cause after a power cycle, and the one
    /// that means "nothing unexplained happened".
    PowerOn,
    /// The watchdog expired. Firmware arms one during boot; if this is what a
    /// post-`halt` reboot reports, the answer is that it was never disarmed.
    Watchdog,
    /// A software reset was requested — `PM_RSTC` written with the password.
    /// Nothing in this kernel does that, so seeing it means the firmware did.
    Software,
    /// A debug reset, from the JTAG/debug path.
    Debug,
    /// The register reported no cause bit at all. Not the same as `PowerOn`:
    /// a warm reset that latched nothing, or a block that is not modelled and
    /// reads back zero. Kept distinct so an absent register cannot be read as
    /// a clean power cycle — the value of this module is precisely that it
    /// does not manufacture an answer.
    None,
}

/// Power-on reset happened (`HADPOR`).
pub const HADPOR: u32 = 0x0000_1000;
/// Watchdog reset: hard, full, request.
pub const HADWR: u32 = 0x0000_0070;
/// Software reset: hard, full, request.
pub const HADSR: u32 = 0x0000_0700;
/// Debug reset: hard, full, request.
pub const HADDR: u32 = 0x0000_0007;

/// Decode the cause of the last reset from a `PM_RSTS` value.
///
/// Ordered most-specific first. More than one flavour of the same cause can be
/// set at once, and a watchdog reset also sets bits a power-on would; the
/// order below is what makes the answer a single cause rather than a set.
///
/// ```
/// use kernel_core::reset::{ResetCause, cause};
///
/// // A clean power cycle, which is what a healthy board reports.
/// assert_eq!(cause(0x0000_1000), ResetCause::PowerOn);
///
/// // The watchdog fired. This is the reading that would explain a board
/// // rebooting itself after the kernel halted.
/// assert_eq!(cause(0x0000_0020), ResetCause::Watchdog);
///
/// // A register that latched nothing makes no claim, and must not be
/// // allowed to become the claim "the board powered on".
/// assert_eq!(cause(0), ResetCause::None);
/// ```
#[inline]
pub const fn cause(rsts: u32) -> ResetCause {
    if rsts & HADWR != 0 {
        ResetCause::Watchdog
    } else if rsts & HADSR != 0 {
        ResetCause::Software
    } else if rsts & HADDR != 0 {
        ResetCause::Debug
    } else if rsts & HADPOR != 0 {
        ResetCause::PowerOn
    } else {
        ResetCause::None
    }
}

/// The partition the firmware was told to boot from, or `None` if the field is
/// the "no partition selected" pattern.
///
/// Six two-bit-interleaved fields in bits 0..11 share the register with the
/// cause bits above, which is why a naive `rsts != 0` test cannot be used to
/// mean "some reset cause was recorded".
#[inline]
pub const fn partition(rsts: u32) -> u32 {
    let mut partition = 0;
    let mut bit = 0;
    while bit < 6 {
        partition |= ((rsts >> (bit * 2)) & 1) << bit;
        bit += 1;
    }
    partition
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_power_cycle_is_recognised() {
        assert_eq!(cause(HADPOR), ResetCause::PowerOn);
    }

    #[test]
    fn every_flavour_of_a_cause_reports_that_cause() {
        // Request, full and hard are three ways for the same thing to have
        // happened. Testing only one of the three would leave two thirds of
        // each mask unexercised, and the masks are where a typo lives.
        for bit in [0x10, 0x20, 0x40] {
            assert_eq!(cause(bit), ResetCause::Watchdog, "watchdog bit {bit:#x}");
        }
        for bit in [0x100, 0x200, 0x400] {
            assert_eq!(cause(bit), ResetCause::Software, "software bit {bit:#x}");
        }
        for bit in [0x1, 0x2, 0x4] {
            assert_eq!(cause(bit), ResetCause::Debug, "debug bit {bit:#x}");
        }
    }

    #[test]
    fn a_watchdog_reset_that_also_set_power_on_reads_as_a_watchdog() {
        // The case the ordering exists for: the board really was reset by the
        // watchdog, and reporting `PowerOn` would answer the question wrongly
        // in the one direction that matters.
        assert_eq!(cause(HADPOR | 0x20), ResetCause::Watchdog);
        assert_eq!(cause(HADPOR | 0x200), ResetCause::Software);
    }

    #[test]
    fn nothing_set_is_not_the_same_as_a_power_on() {
        // Calling an empty register `PowerOn` would manufacture an answer out
        // of a block that said nothing, which is the failure this whole module
        // exists to avoid.
        assert_eq!(cause(0), ResetCause::None);
        assert_ne!(cause(0), ResetCause::PowerOn);
    }

    #[test]
    fn the_partition_field_does_not_leak_into_the_cause() {
        // Partition bits live in 0..11 and overlap the debug-reset mask, so a
        // non-zero partition must not be read as a reset cause on its own.
        // Partition 0 with only the power-on bit is the ordinary case.
        assert_eq!(partition(HADPOR), 0);
        assert_eq!(cause(HADPOR), ResetCause::PowerOn);
    }

    #[test]
    fn the_partition_field_is_six_interleaved_bits() {
        // Bit 2n of the register is bit n of the partition number.
        assert_eq!(partition(0), 0);
        assert_eq!(partition(0b01), 1);
        assert_eq!(partition(0b0100), 2);
        assert_eq!(partition(0b0101), 3);
        // All six set is the firmware's "no partition" pattern, 63.
        assert_eq!(partition(0b0101_0101_0101), 63);
    }
}
