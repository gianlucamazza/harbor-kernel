//! BCM2835/BCM2711 SPI master clock-divisor arithmetic.
//!
//! The controller programs `CLK.CDIV` as an **even** divider of the core
//! clock, or `0` to mean 65 536. Odd values are rounded up by the hardware
//! documentation's software convention; we produce a legal encoding so the
//! driver never writes a value the block treats as "almost what you wanted".
//!
//! Reference: BCM2835 ARM Peripherals, SPI section (carried forward on
//! BCM2711). The pure math lives here so the host tests the edge cases; the
//! driver only writes the register.

/// Upper legal even CDIV value written as a non-zero encoding.
pub const CDIV_MAX_EVEN: u32 = 65_534;
/// Hardware encoding for a divide-by-65536.
pub const CDIV_ENCODE_65536: u32 = 0;

/// Why a target SPI rate cannot be programmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockDivError {
    /// Core or target frequency was zero.
    ZeroFrequency,
    /// Even after maximum division the SPI clock would still be faster than
    /// requested — the core is too fast / target too slow for this block.
    TargetTooSlow { core_hz: u32, target_hz: u32 },
}

/// Compute the `CLK.CDIV` encoding for a SPI bit clock at most `target_hz`.
///
/// Rounds the divider **up** (and then to the next even value) so the
/// programmed rate never exceeds the caller's ceiling — important for panel
/// controllers with a hard Fmax.
///
/// Returns `CDIV_ENCODE_65536` (`0`) when the required divider is 65 536.
pub const fn clock_divisor(core_hz: u32, target_hz: u32) -> Result<u32, ClockDivError> {
    if core_hz == 0 || target_hz == 0 {
        return Err(ClockDivError::ZeroFrequency);
    }

    // Ceil division: smallest integer div with core/div <= target is wrong;
    // we want core/div <= target, i.e. div >= ceil(core/target).
    let mut div = core_hz.div_ceil(target_hz);

    // Hardware minimum useful divider is 2 (div 0/1 are special / odd).
    if div < 2 {
        div = 2;
    }

    // Range check *before* rounding, not after. Rounding an out-of-range
    // divider up can wrap: `core_hz = u32::MAX, target_hz = 1` gives an odd
    // `div` of `u32::MAX`, and `div + 1` wraps to 0 — which this function then
    // returns as `CDIV_ENCODE_65536`, turning the fastest clock the caller can
    // ask for into the slowest the block can produce. Refusing first makes the
    // increment below unreachable for any `div` that could overflow: the
    // largest odd value reaching it is 65 535.
    if div > 65_536 {
        return Err(ClockDivError::TargetTooSlow { core_hz, target_hz });
    }

    // Force even. Safe by the check above: 65 535 + 1 == 65 536, the maximum.
    if div % 2 != 0 {
        div += 1;
    }

    if div == 65_536 {
        Ok(CDIV_ENCODE_65536)
    } else {
        Ok(div)
    }
}

/// SPI bit clock that results from programming `cdiv` at `core_hz`.
///
/// `cdiv == 0` means divide by 65 536. Odd non-zero values are treated as the
/// next even value above, matching the usual software convention for this
/// block.
pub const fn effective_hz(core_hz: u32, cdiv: u32) -> u32 {
    let div = if cdiv == 0 {
        65_536
    } else if cdiv % 2 != 0 {
        cdiv + 1
    } else {
        cdiv
    };
    if div == 0 {
        return 0;
    }
    core_hz / div
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 500 MHz core (Pi 4 `core_freq_min=500`) toward a 16 MHz panel ceiling.
    #[test]
    fn waveshare_class_16mhz_ceiling() {
        let cdiv = clock_divisor(500_000_000, 16_000_000).expect("programmable");
        assert_eq!(cdiv, 32);
        assert!(effective_hz(500_000_000, cdiv) <= 16_000_000);
        assert_eq!(effective_hz(500_000_000, cdiv), 15_625_000);
    }

    #[test]
    fn already_even_ceil_is_preserved() {
        // 100 / 25 = 4 exactly.
        assert_eq!(clock_divisor(100, 25), Ok(4));
    }

    #[test]
    fn odd_ceil_rounds_up_to_even() {
        // ceil(100/30) = 4 — already even.
        assert_eq!(clock_divisor(100, 30), Ok(4));
        // ceil(100/40) = 3 → 4.
        assert_eq!(clock_divisor(100, 40), Ok(4));
        assert!(effective_hz(100, 4) <= 40);
    }

    #[test]
    fn never_exceeds_target() {
        let core = 500_000_000u32;
        for target in [1_000_000u32, 8_000_000, 16_000_000, 32_000_000, 50_000_000] {
            let cdiv = clock_divisor(core, target).expect("in range");
            assert!(
                effective_hz(core, cdiv) <= target,
                "cdiv={cdiv} effective={} > target={target}",
                effective_hz(core, cdiv)
            );
        }
    }

    #[test]
    fn max_divider_encodes_as_zero() {
        // Need div 65536: core/target = 65536 → target = core/65536.
        let core = 65_536u32 * 100;
        let target = 100;
        assert_eq!(clock_divisor(core, target), Ok(CDIV_ENCODE_65536));
        assert_eq!(effective_hz(core, 0), 100);
    }

    #[test]
    fn target_slower_than_max_division_is_refused() {
        // Even /65536 is still too fast.
        assert_eq!(
            clock_divisor(65_536 * 200, 100),
            Err(ClockDivError::TargetTooSlow {
                core_hz: 65_536 * 200,
                target_hz: 100,
            })
        );
    }

    #[test]
    fn zero_frequencies_are_refused() {
        assert_eq!(
            clock_divisor(0, 1_000_000),
            Err(ClockDivError::ZeroFrequency)
        );
        assert_eq!(
            clock_divisor(500_000_000, 0),
            Err(ClockDivError::ZeroFrequency)
        );
    }

    #[test]
    fn target_at_or_above_core_uses_minimum_divider() {
        // Cannot go faster than core/2 on this block.
        assert_eq!(clock_divisor(100, 100), Ok(2));
        assert_eq!(effective_hz(100, 2), 50);
    }

    /// The widest ratio a `u32` pair can express. Rounding this up to an even
    /// divider wraps, and the wrapped value `0` is a *legal* encoding meaning
    /// divide-by-65536 — so the failure is not a panic but a plausible answer
    /// that is maximally wrong: the fastest request becomes the slowest clock.
    #[test]
    fn the_widest_possible_ratio_is_refused_rather_than_wrapping() {
        assert_eq!(
            clock_divisor(u32::MAX, 1),
            Err(ClockDivError::TargetTooSlow {
                core_hz: u32::MAX,
                target_hz: 1,
            })
        );
    }

    /// The boundary the check above must not move: an odd divider of 65 535
    /// still has an even value in range, and must round up rather than refuse.
    #[test]
    fn the_largest_odd_divider_in_range_still_rounds_up() {
        // ceil(65_535 * 7 / 7) = 65_535, odd, one below the maximum.
        assert_eq!(clock_divisor(65_535 * 7, 7), Ok(CDIV_ENCODE_65536));
        assert_eq!(effective_hz(65_535 * 7, CDIV_ENCODE_65536), 6);
    }
}
