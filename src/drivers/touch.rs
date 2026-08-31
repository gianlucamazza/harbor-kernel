//! Board-independent XPT2046/ADS7846 transaction helper.

use kernel_core::touch::{RawSample, decode_adc};

use super::spi::SpiDevice;

/// XPT2046 controller over an 8-bit SPI device.
pub struct Xpt2046<D> {
    device: D,
}

impl<D> Xpt2046<D>
where
    D: SpiDevice,
{
    pub const fn new(device: D) -> Self {
        Self { device }
    }

    /// Read one bounded X/Y/pressure sample.
    pub fn sample(&mut self) -> Result<RawSample, D::Error> {
        let mut x = [0u8; 2];
        let mut y = [0u8; 2];
        let mut z = [0u8; 2];
        self.read_channel(0xD0, &mut x)?;
        self.read_channel(0x90, &mut y)?;
        self.read_channel(0xB0, &mut z)?;
        Ok(RawSample {
            x: decode_adc(x[0], x[1]),
            y: decode_adc(y[0], y[1]),
            pressure: decode_adc(z[0], z[1]),
        })
    }

    fn read_channel(&mut self, command: u8, out: &mut [u8; 2]) -> Result<(), D::Error> {
        let tx = [command, 0, 0];
        let mut rx = [0u8; 3];
        self.device.transfer(&mut rx, &tx)?;
        out.copy_from_slice(&rx[1..]);
        Ok(())
    }
}
