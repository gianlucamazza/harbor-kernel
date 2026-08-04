//! Pure display arithmetic: RGB565 packing and declarative panel init ops.
//!
//! The panel driver walks [`InitOp`] lists and streams pixels; this module
//! only owns the host-testable encoding.

/// 16-bit colour for ILI9486 `COLMOD=0x55` (RGB 5-6-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb565(pub u16);

impl Rgb565 {
    /// Pack 8-bit channels (top bits kept).
    pub const fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        let r5 = (r as u16) >> 3;
        let g6 = (g as u16) >> 2;
        let b5 = (b as u16) >> 3;
        Self((r5 << 11) | (g6 << 5) | b5)
    }

    /// Big-endian wire order used by ILI9486 over 8-bit SPI.
    #[inline]
    pub const fn to_be_bytes(self) -> [u8; 2] {
        self.0.to_be_bytes()
    }

    pub const BLACK: Self = Self(0x0000);
    pub const WHITE: Self = Self(0xFFFF);
    pub const RED: Self = Self::from_rgb8(0xE0, 0x20, 0x20);
    pub const GREEN: Self = Self::from_rgb8(0x20, 0xC0, 0x40);
    pub const BLUE: Self = Self::from_rgb8(0x20, 0x40, 0xE0);
    /// Dark navy — distinct from the unprogrammed white backlight field.
    pub const HARBOR: Self = Self::from_rgb8(0x0A, 0x14, 0x28);
}

/// One step in a panel power-on / mode table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitOp {
    /// DC low, then this command byte.
    Cmd(u8),
    /// DC high, then these data bytes (may be empty only for structure; prefer
    /// omitting the step).
    Data(&'static [u8]),
    /// Wall-time wait after the previous step.
    DelayMs(u16),
}

/// MIPI DCS / ILI9486 commands we use (datasheet + MIPI DCS 1.02).
pub mod cmd {
    pub const NOP: u8 = 0x00;
    pub const SWRESET: u8 = 0x01;
    pub const SLPIN: u8 = 0x10;
    pub const SLPOUT: u8 = 0x11;
    pub const DISPOFF: u8 = 0x28;
    pub const DISPON: u8 = 0x29;
    pub const CASET: u8 = 0x2A;
    pub const PASET: u8 = 0x2B;
    pub const RAMWR: u8 = 0x2C;
    pub const MADCTL: u8 = 0x36;
    pub const COLMOD: u8 = 0x3A;
    /// Interface Mode Control (ILI9486).
    pub const IFMODE: u8 = 0xB0;
    /// Power Control 3.
    pub const PWCTR3: u8 = 0xC2;
    /// VCOM Control 1.
    pub const VMCTR1: u8 = 0xC5;
    /// Positive gamma.
    pub const PGAMCTRL: u8 = 0xE0;
    /// Negative gamma.
    pub const NGAMCTRL: u8 = 0xE1;
    /// Digital gamma.
    pub const DGAMCTRL: u8 = 0xE2;
}

/// MADCTL bits (memory data access control).
pub mod madctl {
    pub const MY: u8 = 1 << 7;
    pub const MX: u8 = 1 << 6;
    pub const MV: u8 = 1 << 5;
    pub const ML: u8 = 1 << 4;
    pub const BGR: u8 = 1 << 3;
    pub const MH: u8 = 1 << 2;
}

/// Encode a column or page address window as four CASET/PASET data bytes.
pub const fn address_window_bytes(start: u16, end: u16) -> [u8; 4] {
    [
        (start >> 8) as u8,
        (start & 0xFF) as u8,
        (end >> 8) as u8,
        (end & 0xFF) as u8,
    ]
}

/// Pixel count for a full frame.
pub const fn frame_pixels(width: u16, height: u16) -> u32 {
    (width as u32) * (height as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_black_and_white() {
        assert_eq!(Rgb565::BLACK.0, 0);
        assert_eq!(Rgb565::WHITE.0, 0xFFFF);
    }

    #[test]
    fn rgb565_red_channel_in_high_bits() {
        let r = Rgb565::from_rgb8(0xF8, 0, 0);
        assert_eq!(r.0 & 0xF800, 0xF800);
        assert_eq!(r.0 & 0x07FF, 0);
    }

    #[test]
    fn be_wire_order() {
        assert_eq!(Rgb565(0x1234).to_be_bytes(), [0x12, 0x34]);
    }

    #[test]
    fn address_window_splits_u16() {
        assert_eq!(address_window_bytes(0, 479), [0, 0, 0x01, 0xDF]);
        assert_eq!(address_window_bytes(10, 20), [0, 10, 0, 20]);
    }

    #[test]
    fn frame_pixel_count_waveshare_landscape() {
        assert_eq!(frame_pixels(480, 320), 153_600);
    }

    #[test]
    fn init_ops_are_plain_data() {
        let seq = [
            InitOp::Cmd(cmd::SLPOUT),
            InitOp::DelayMs(5),
            InitOp::Cmd(cmd::COLMOD),
            InitOp::Data(&[0x55]),
        ];
        assert_eq!(seq.len(), 4);
    }
}
