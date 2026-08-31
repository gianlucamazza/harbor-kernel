//! Pure XPT2046/ADS7846 sample decoding and coordinate normalization.

use crate::ui::{SCREEN_HEIGHT, SCREEN_WIDTH, TouchEvent, TouchKind};

/// One decoded 12-bit resistive touch sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawSample {
    pub x: u16,
    pub y: u16,
    pub pressure: u16,
}

/// Calibration constants for the common landscape Waveshare panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calibration {
    pub x_min: u16,
    pub x_max: u16,
    pub y_min: u16,
    pub y_max: u16,
}

impl Calibration {
    pub const DEFAULT: Self = Self {
        x_min: 200,
        x_max: 3900,
        y_min: 200,
        y_max: 3900,
    };

    /// Convert a raw sample into panel coordinates, clamping bad input.
    pub const fn normalize(self, sample: RawSample) -> (u16, u16) {
        let x = scale(sample.x, self.x_min, self.x_max, SCREEN_WIDTH - 1);
        let y = scale(sample.y, self.y_min, self.y_max, SCREEN_HEIGHT - 1);
        (x, y)
    }
}

/// Decode a 12-bit ADC response from two SPI bytes.
pub const fn decode_adc(hi: u8, lo: u8) -> u16 {
    (u16::from_be_bytes([hi, lo]) >> 3) & 0x0FFF
}

/// Build a normalized touch event.
pub const fn event(kind: TouchKind, sample: RawSample, calibration: Calibration) -> TouchEvent {
    let (x, y) = calibration.normalize(sample);
    TouchEvent { kind, x, y }
}

const fn scale(value: u16, min: u16, max: u16, extent: u16) -> u16 {
    if value <= min || max <= min {
        return 0;
    }
    if value >= max {
        return extent;
    }
    let numerator = (value - min) as u32 * extent as u32;
    (numerator / (max - min) as u32) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adc_decode_keeps_twelve_bits() {
        assert_eq!(decode_adc(0x12, 0x30), 0x246);
    }

    #[test]
    fn normalization_clamps_and_scales() {
        let c = Calibration::DEFAULT;
        assert_eq!(
            c.normalize(RawSample {
                x: 200,
                y: 200,
                pressure: 1
            }),
            (0, 0)
        );
        assert_eq!(
            c.normalize(RawSample {
                x: 3900,
                y: 3900,
                pressure: 1
            }),
            (479, 319)
        );
        assert_eq!(
            c.normalize(RawSample {
                x: 2050,
                y: 2050,
                pressure: 1
            }),
            (239, 159)
        );
        assert_eq!(
            c.normalize(RawSample {
                x: 0,
                y: 5000,
                pressure: 1
            }),
            (0, 319)
        );
    }
}
