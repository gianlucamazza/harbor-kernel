//! Board-independent XPT2046/ADS7846 transaction helper.

use kernel_core::touch::{RawSample, decode_adc, median3, pressure_from_z};

use super::spi::SpiDevice;

/// XPT2046 controller over an 8-bit SPI device.
pub struct Xpt2046<D> {
    device: D,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeDevice {
        commands: [u8; 4],
        seen: usize,
        fail: bool,
    }

    impl SpiDevice for FakeDevice {
        type Error = ();

        fn write(&mut self, _words: &[u8]) -> Result<(), Self::Error> {
            Ok(())
        }

        fn transfer(&mut self, read: &mut [u8], words: &[u8]) -> Result<(), Self::Error> {
            if self.fail || read.len() != 3 || words.len() != 3 {
                return Err(());
            }
            let expected = [0xD0, 0x90, 0xB0, 0xC0][self.seen];
            assert_eq!(words[0], expected);
            self.commands[self.seen] = words[0];
            self.seen += 1;
            // 0x8000 decodes to 0x1000, 0x4000 to 0x0800.  Z1 > Z2
            // produces a non-zero pressure for the X sample.
            let value = match words[0] {
                0xD0 => 0x8000,
                0x90 => 0x8000,
                0xB0 => 0x8000,
                0xC0 => 0x4000,
                _ => 0,
            };
            read[0] = 0;
            read[1..].copy_from_slice(&value.to_be_bytes());
            Ok(())
        }

        fn transfer_in_place(&mut self, _words: &mut [u8]) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn sample_uses_the_complete_xy_z1_z2_sequence() {
        let fake = FakeDevice {
            commands: [0; 4],
            seen: 0,
            fail: false,
        };
        let mut controller = Xpt2046::new(fake);
        let sample = controller.sample().expect("fake transfer succeeds");
        assert_eq!(sample.x, 0x1000);
        assert_eq!(sample.y, 0x1000);
        assert_eq!(sample.pressure, 0x1000);
        assert_eq!(controller.device.seen, 4);
        assert_eq!(controller.device.commands, [0xD0, 0x90, 0xB0, 0xC0]);
    }

    #[test]
    fn sample_propagates_spi_failure() {
        let fake = FakeDevice {
            commands: [0; 4],
            seen: 0,
            fail: true,
        };
        let mut controller = Xpt2046::new(fake);
        assert_eq!(controller.sample(), Err(()));
    }
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
        let mut z1 = [0u8; 2];
        let mut z2 = [0u8; 2];
        self.read_channel(0xD0, &mut x)?;
        self.read_channel(0x90, &mut y)?;
        self.read_channel(0xB0, &mut z1)?;
        self.read_channel(0xC0, &mut z2)?;
        let x = decode_adc(x[0], x[1]);
        let z1 = decode_adc(z1[0], z1[1]);
        let z2 = decode_adc(z2[0], z2[1]);
        Ok(RawSample {
            x,
            y: decode_adc(y[0], y[1]),
            pressure: pressure_from_z(z1, z2, x),
        })
    }

    /// Read three samples and use the median to reject ADC spikes.
    pub fn sample_filtered(&mut self) -> Result<RawSample, D::Error> {
        let a = self.sample()?;
        let b = self.sample()?;
        let c = self.sample()?;
        Ok(RawSample {
            x: median3(a.x, b.x, c.x),
            y: median3(a.y, b.y, c.y),
            pressure: median3(a.pressure, b.pressure, c.pressure),
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
