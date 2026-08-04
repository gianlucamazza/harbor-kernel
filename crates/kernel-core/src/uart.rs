//! PL011 baud-rate divisor math (ARM DDI 0183).
//!
//! `baud = clock / (16 * (IBRD + FBRD/64))`, so the 6.6 fixed-point divisor is
//! `divisor_x64 = 64 * clock / (16 * baud) = 4 * clock / baud`.

/// Line rate request: reference clock and desired baud.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaudConfig {
    /// UART reference clock in Hz (board-specific).
    pub clock_hz: u32,
    /// Desired baud rate.
    pub baud: u32,
}

/// Programmed divisors: `IBRD` is 16-bit, `FBRD` is 6-bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divisors {
    pub ibrd: u32,
    pub fbrd: u32,
}

impl BaudConfig {
    /// Compute the integer and fractional divisors.
    ///
    /// Returns `None` when the request cannot be programmed: zero clock or
    /// baud, or an integer divisor that does not fit `IBRD`.
    pub const fn divisors(self) -> Option<Divisors> {
        if self.clock_hz == 0 || self.baud == 0 {
            return None;
        }

        // 64-bit: `4 * clock_hz` overflows u32 above ~1.07 GHz.
        let clock = self.clock_hz as u64;
        let baud = self.baud as u64;

        // Round to nearest rather than truncate: the fractional divisor has
        // only 6 bits, so truncation doubles the worst-case rate error.
        let divisor_x64 = (4 * clock + baud / 2) / baud;

        let ibrd = divisor_x64 >> 6;
        let fbrd = divisor_x64 & 0x3F;

        // IBRD is 16 bits and must be non-zero (0 stops the baud generator).
        if ibrd == 0 || ibrd > 0xFFFF {
            return None;
        }

        Some(Divisors {
            ibrd: ibrd as u32,
            fbrd: fbrd as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The board case: 48 MHz / 115200. `4 * 48e6 / 115200 = 1666.67`, which
    /// rounds to 1667 → IBRD 26, FBRD 3. Truncating to 1666 gives FBRD 2 and a
    /// worse rate error, so rounding is the behaviour we require.
    #[test]
    fn board_console_rate_rounds_to_nearest() {
        let d = BaudConfig {
            clock_hz: 48_000_000,
            baud: 115_200,
        }
        .divisors()
        .expect("48MHz/115200 is programmable");
        assert_eq!(d, Divisors { ibrd: 26, fbrd: 3 });
    }

    #[test]
    fn exact_division_has_no_fractional_part() {
        // 4 * 1_843_200 / 115_200 = 64 exactly → IBRD 1, FBRD 0.
        let d = BaudConfig {
            clock_hz: 1_843_200,
            baud: 115_200,
        }
        .divisors()
        .unwrap();
        assert_eq!(d, Divisors { ibrd: 1, fbrd: 0 });
    }

    #[test]
    fn zero_baud_is_rejected_not_a_division_by_zero() {
        assert_eq!(
            BaudConfig {
                clock_hz: 48_000_000,
                baud: 0
            }
            .divisors(),
            None
        );
    }

    #[test]
    fn zero_clock_is_rejected() {
        assert_eq!(
            BaudConfig {
                clock_hz: 0,
                baud: 115_200
            }
            .divisors(),
            None
        );
    }

    /// `4 * clock_hz` overflows `u32` above ~1.07 GHz. A plausible future
    /// reference clock must not silently wrap into a bogus divisor.
    #[test]
    fn high_reference_clock_does_not_overflow() {
        let d = BaudConfig {
            clock_hz: 2_000_000_000,
            baud: 115_200,
        }
        .divisors();
        // 4 * 2e9 / 115200 = 69444.4 → 69444 → IBRD 1085 (fits), FBRD 4.
        assert_eq!(
            d,
            Some(Divisors {
                ibrd: 1085,
                fbrd: 4
            })
        );
    }

    /// IBRD is 16 bits: a divisor beyond that cannot be programmed.
    #[test]
    fn unprogrammable_divisor_is_rejected() {
        assert_eq!(
            BaudConfig {
                clock_hz: 500_000_000,
                baud: 100
            }
            .divisors(),
            None
        );
    }
}
