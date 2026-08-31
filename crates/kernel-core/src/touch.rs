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

/// Versioned, fixed-size calibration record for persistent board storage.
pub const CALIBRATION_RECORD_VERSION: u8 = 1;
pub const CALIBRATION_RECORD_LEN: usize = 11;
pub const ADC_MAX: u16 = 0x0fff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationCodecError {
    BufferLength,
    UnsupportedVersion,
    Checksum,
    InvalidCalibration,
}

/// Minimum pressure accepted for a calibration point.
pub const CALIBRATION_MIN_PRESSURE: u16 = 32;

/// Fixed-capacity collection of corner samples used by a calibration UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalibrationWizard {
    samples: [Option<RawSample>; 4],
    count: u8,
}

impl CalibrationWizard {
    pub const fn new() -> Self {
        Self {
            samples: [None; 4],
            count: 0,
        }
    }

    pub const fn count(self) -> usize {
        self.count as usize
    }

    pub const fn is_complete(self) -> bool {
        self.count as usize == self.samples.len()
    }

    /// Add one accepted raw sample in the UI's prescribed corner order.
    pub fn push(&mut self, sample: RawSample) -> bool {
        if self.is_complete() || sample.pressure < CALIBRATION_MIN_PRESSURE {
            return false;
        }
        self.samples[self.count as usize] = Some(sample);
        self.count += 1;
        true
    }

    /// Derive a calibration from the collected corner samples.
    pub fn finish(self) -> Result<Calibration, CalibrationCodecError> {
        if !self.is_complete() {
            return Err(CalibrationCodecError::InvalidCalibration);
        }
        let first = self.samples[0].unwrap();
        let mut x_min = first.x;
        let mut x_max = first.x;
        let mut y_min = first.y;
        let mut y_max = first.y;
        for sample in self.samples[1..].iter().flatten() {
            x_min = x_min.min(sample.x);
            x_max = x_max.max(sample.x);
            y_min = y_min.min(sample.y);
            y_max = y_max.max(sample.y);
        }
        let calibration = Calibration {
            x_min,
            x_max,
            y_min,
            y_max,
        };
        if calibration.is_valid() {
            Ok(calibration)
        } else {
            Err(CalibrationCodecError::InvalidCalibration)
        }
    }
}

impl Default for CalibrationWizard {
    fn default() -> Self {
        Self::new()
    }
}

/// Fixed-point orientation transform applied after calibration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Normal,
    Rotate180,
    SwapAxes,
    MirrorX,
    MirrorY,
}

impl Calibration {
    pub const DEFAULT: Self = Self {
        x_min: 200,
        x_max: 3900,
        y_min: 200,
        y_max: 3900,
    };

    pub const fn is_valid(self) -> bool {
        self.x_max <= ADC_MAX
            && self.y_max <= ADC_MAX
            && self.x_max.saturating_sub(self.x_min) >= 32
            && self.y_max.saturating_sub(self.y_min) >= 32
    }

    /// Encode a calibration record without allocation or storage side effects.
    pub fn encode(self, out: &mut [u8]) -> Result<(), CalibrationCodecError> {
        if out.len() != CALIBRATION_RECORD_LEN {
            return Err(CalibrationCodecError::BufferLength);
        }
        if !self.is_valid() {
            return Err(CalibrationCodecError::InvalidCalibration);
        }
        out[0] = CALIBRATION_RECORD_VERSION;
        put_u16(&mut out[1..3], self.x_min);
        put_u16(&mut out[3..5], self.x_max);
        put_u16(&mut out[5..7], self.y_min);
        put_u16(&mut out[7..9], self.y_max);
        let crc = crc16(&out[..9]);
        put_u16(&mut out[9..11], crc);
        Ok(())
    }

    /// Decode and validate a calibration record read from board storage.
    pub fn decode(bytes: &[u8]) -> Result<Self, CalibrationCodecError> {
        if bytes.len() != CALIBRATION_RECORD_LEN {
            return Err(CalibrationCodecError::BufferLength);
        }
        if bytes[0] != CALIBRATION_RECORD_VERSION {
            return Err(CalibrationCodecError::UnsupportedVersion);
        }
        if get_u16(&bytes[9..11]) != crc16(&bytes[..9]) {
            return Err(CalibrationCodecError::Checksum);
        }
        let calibration = Self {
            x_min: get_u16(&bytes[1..3]),
            x_max: get_u16(&bytes[3..5]),
            y_min: get_u16(&bytes[5..7]),
            y_max: get_u16(&bytes[7..9]),
        };
        if !calibration.is_valid() {
            return Err(CalibrationCodecError::InvalidCalibration);
        }
        Ok(calibration)
    }

    /// Convert a raw sample into panel coordinates, clamping bad input.
    pub const fn normalize(self, sample: RawSample) -> (u16, u16) {
        let x = scale(sample.x, self.x_min, self.x_max, SCREEN_WIDTH - 1);
        let y = scale(sample.y, self.y_min, self.y_max, SCREEN_HEIGHT - 1);
        (x, y)
    }
}

fn put_u16(out: &mut [u8], value: u16) {
    let bytes = value.to_le_bytes();
    out.copy_from_slice(&bytes);
}

fn get_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff;
    for &byte in bytes {
        crc ^= (byte as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
            bit += 1;
        }
    }
    crc
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

/// Build an event with the panel's explicit orientation transform.
pub const fn event_oriented(
    kind: TouchKind,
    sample: RawSample,
    calibration: Calibration,
    orientation: Orientation,
) -> TouchEvent {
    let (mut x, mut y) = calibration.normalize(sample);
    match orientation {
        Orientation::Normal => {}
        Orientation::Rotate180 => {
            x = SCREEN_WIDTH - 1 - x;
            y = SCREEN_HEIGHT - 1 - y;
        }
        Orientation::SwapAxes => {
            let old_x = x;
            x = clamp(y, SCREEN_WIDTH - 1);
            y = clamp(old_x, SCREEN_HEIGHT - 1);
        }
        Orientation::MirrorX => x = SCREEN_WIDTH - 1 - x,
        Orientation::MirrorY => y = SCREEN_HEIGHT - 1 - y,
    }
    TouchEvent { kind, x, y }
}

/// Median of three bounded ADC values.
pub const fn median3(a: u16, b: u16, c: u16) -> u16 {
    if (a <= b && b <= c) || (c <= b && b <= a) {
        b
    } else if (b <= a && a <= c) || (c <= a && a <= b) {
        a
    } else {
        c
    }
}

/// Calculate the XPT2046 touch-resistance proxy from Z1/Z2.
pub const fn pressure_from_z(z1: u16, z2: u16, x: u16) -> u16 {
    if z2 == 0 || z1 <= z2 {
        return 0;
    }
    let value = (x as u32 * (z1 - z2) as u32) / z2 as u32;
    if value > u16::MAX as u32 {
        u16::MAX
    } else {
        value as u16
    }
}

const fn clamp(value: u16, max: u16) -> u16 {
    if value > max { max } else { value }
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

    #[test]
    fn calibration_validity_and_orientation_are_explicit() {
        assert!(Calibration::DEFAULT.is_valid());
        assert!(
            !Calibration {
                x_min: 2,
                x_max: 2,
                ..Calibration::DEFAULT
            }
            .is_valid()
        );
        assert!(
            !Calibration {
                x_max: ADC_MAX + 1,
                ..Calibration::DEFAULT
            }
            .is_valid()
        );
        assert!(
            !Calibration {
                y_min: 4000,
                y_max: 4020,
                ..Calibration::DEFAULT
            }
            .is_valid()
        );
        let sample = RawSample {
            x: 200,
            y: 200,
            pressure: 1,
        };
        assert_eq!(
            event_oriented(
                TouchKind::Down,
                sample,
                Calibration::DEFAULT,
                Orientation::Rotate180
            )
            .x,
            479
        );
        assert_eq!(
            event_oriented(
                TouchKind::Down,
                sample,
                Calibration::DEFAULT,
                Orientation::Rotate180
            )
            .y,
            319
        );
    }

    #[test]
    fn median_rejects_one_adc_spike() {
        assert_eq!(median3(100, 3900, 105), 105);
        assert_eq!(median3(3900, 100, 105), 105);
    }

    #[test]
    fn pressure_uses_z1_z2_and_refuses_invalid_values() {
        assert_eq!(pressure_from_z(300, 200, 1000), 500);
        assert_eq!(pressure_from_z(100, 200, 1000), 0);
        assert_eq!(pressure_from_z(0, 0, 1000), 0);
    }

    #[test]
    fn calibration_record_round_trips_with_crc() {
        let calibration = Calibration {
            x_min: 210,
            x_max: 3880,
            y_min: 225,
            y_max: 3860,
        };
        let mut record = [0; CALIBRATION_RECORD_LEN];
        calibration.encode(&mut record).unwrap();
        assert_eq!(Calibration::decode(&record), Ok(calibration));
    }

    #[test]
    fn calibration_record_rejects_corruption_and_bad_version() {
        let mut record = [0; CALIBRATION_RECORD_LEN];
        Calibration::DEFAULT.encode(&mut record).unwrap();
        record[4] ^= 1;
        assert_eq!(
            Calibration::decode(&record),
            Err(CalibrationCodecError::Checksum)
        );
        record[4] ^= 1;
        record[0] = 99;
        assert_eq!(
            Calibration::decode(&record),
            Err(CalibrationCodecError::UnsupportedVersion)
        );
    }

    #[test]
    fn calibration_record_rejects_invalid_shape_and_length() {
        let invalid = Calibration {
            x_min: 4,
            x_max: 4,
            ..Calibration::DEFAULT
        };
        let mut record = [0; CALIBRATION_RECORD_LEN];
        assert_eq!(
            invalid.encode(&mut record),
            Err(CalibrationCodecError::InvalidCalibration)
        );
        assert_eq!(
            Calibration::DEFAULT.encode(&mut [0; 2]),
            Err(CalibrationCodecError::BufferLength)
        );
        assert_eq!(
            Calibration::decode(&[0; 2]),
            Err(CalibrationCodecError::BufferLength)
        );
    }

    #[test]
    fn calibration_wizard_requires_pressure_and_four_spread_points() {
        let mut wizard = CalibrationWizard::new();
        assert!(!wizard.push(RawSample {
            x: 200,
            y: 200,
            pressure: 31
        }));
        for sample in [
            RawSample {
                x: 210,
                y: 220,
                pressure: 100,
            },
            RawSample {
                x: 3880,
                y: 225,
                pressure: 100,
            },
            RawSample {
                x: 215,
                y: 3860,
                pressure: 100,
            },
            RawSample {
                x: 3870,
                y: 3850,
                pressure: 100,
            },
        ] {
            assert!(wizard.push(sample));
        }
        assert!(wizard.is_complete());
        assert_eq!(wizard.count(), 4);
        assert_eq!(wizard.finish().unwrap().x_min, 210);
        assert!(!wizard.push(RawSample {
            x: 200,
            y: 200,
            pressure: 100
        }));
    }

    #[test]
    fn calibration_wizard_refuses_incomplete_or_degenerate_points() {
        let mut incomplete = CalibrationWizard::new();
        assert_eq!(
            incomplete.finish(),
            Err(CalibrationCodecError::InvalidCalibration)
        );
        for _ in 0..4 {
            assert!(incomplete.push(RawSample {
                x: 100,
                y: 100,
                pressure: 100,
            }));
        }
        assert_eq!(
            incomplete.finish(),
            Err(CalibrationCodecError::InvalidCalibration)
        );
    }
}
