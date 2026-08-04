//! ILI9486 TFT panel over SPI (MIPI DBI-style command/data).
//!
//! Board-agnostic: takes a [`SpiDevice`], a DC pin, and a delay. Init tables
//! are declarative [`InitOp`] lists — datasheet opcodes with fbtft PiScreen
//! values as the documented cross-check for Waveshare-class glass
//! (ADR-0009: datasheet-first, no opaque blob).
//!
//! Pixel path is RGB565, big-endian on the wire, 8-bit SPI words.
//!
//! **CS discipline:** a RAMWR stream must keep chip-select asserted for the
//! whole pixel burst. Toggling CS between chunks ends the write on ILI9486 and
//! leaves the rest of GRAM untouched (classic white band after a partial fill).

extern crate alloc;

use alloc::vec;

use kernel_core::display::{self, InitOp, Rgb565, address_window_bytes, cmd, madctl};

use crate::drivers::delay::DelayNs;
use crate::drivers::pin::OutputPin;
use crate::drivers::spi::SpiDevice;

/// Landscape 480×320 as used by Waveshare-class 3.5″ HATs after MADCTL MV.
pub const WIDTH: u16 = 480;
pub const HEIGHT: u16 = 320;

/// PiScreen / Waveshare-class power and gamma (Linux `fb_ili9486` default
/// sequence). Command bytes are ILI9486 / MIPI DCS; parameter blobs match the
/// open fbtft table used on those HATs.
pub const INIT_PISCREEN: &[InitOp] = &[
    InitOp::Cmd(cmd::IFMODE),
    InitOp::Data(&[0x00]),
    InitOp::Cmd(cmd::SLPOUT),
    InitOp::DelayMs(120),
    InitOp::Cmd(cmd::COLMOD),
    InitOp::Data(&[0x55]), // 16 bpp
    InitOp::Cmd(cmd::PWCTR3),
    InitOp::Data(&[0x44]),
    InitOp::Cmd(cmd::VMCTR1),
    InitOp::Data(&[0x00, 0x00, 0x00, 0x00]),
    InitOp::Cmd(cmd::PGAMCTRL),
    InitOp::Data(&[
        0x0F, 0x1F, 0x1C, 0x0C, 0x0F, 0x08, 0x48, 0x98, 0x37, 0x0A, 0x13, 0x04, 0x11, 0x0D, 0x00,
    ]),
    InitOp::Cmd(cmd::NGAMCTRL),
    InitOp::Data(&[
        0x0F, 0x32, 0x2E, 0x0B, 0x0D, 0x05, 0x47, 0x75, 0x37, 0x06, 0x10, 0x03, 0x24, 0x20, 0x00,
    ]),
    InitOp::Cmd(cmd::DGAMCTRL),
    InitOp::Data(&[
        0x0F, 0x32, 0x2E, 0x0B, 0x0D, 0x05, 0x47, 0x75, 0x37, 0x06, 0x10, 0x03, 0x24, 0x20, 0x00,
    ]),
    // Landscape, BGR order (fbtft rotate=90 + bgr).
    InitOp::Cmd(cmd::MADCTL),
    InitOp::Data(&[madctl::MV | madctl::BGR]),
    InitOp::Cmd(cmd::DISPON),
    InitOp::DelayMs(20),
];

/// Why a panel operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ili9486Error<S, P> {
    Spi(S),
    Pin(P),
}

/// ILI9486 driven over SPI + DC.
pub struct Ili9486<SPI, DC, D> {
    spi: SPI,
    dc: DC,
    delay: D,
    width: u16,
    height: u16,
}

impl<SPI, DC, D> Ili9486<SPI, DC, D>
where
    SPI: SpiDevice,
    DC: OutputPin,
    D: DelayNs,
{
    /// Bind the panel without programming it.
    pub fn new(spi: SPI, dc: DC, delay: D, width: u16, height: u16) -> Self {
        Self {
            spi,
            dc,
            delay,
            width,
            height,
        }
    }

    /// Hardware reset via the RST pin, then run `init` (typically [`INIT_PISCREEN`]).
    pub fn reset_and_init<RST: OutputPin>(
        &mut self,
        rst: &mut RST,
        init: &[InitOp],
    ) -> Result<(), Ili9486Error<SPI::Error, DC::Error>> {
        // Active-low reset pulse (pin errors are ignored only when Infallible).
        let _ = rst.set_low();
        self.delay.delay_ms(20);
        let _ = rst.set_high();
        self.delay.delay_ms(120);
        self.run_init(init)
    }

    /// Execute a declarative init table.
    pub fn run_init(&mut self, init: &[InitOp]) -> Result<(), Ili9486Error<SPI::Error, DC::Error>> {
        for op in init {
            match *op {
                InitOp::Cmd(c) => self.write_cmd(c)?,
                InitOp::Data(bytes) => {
                    if !bytes.is_empty() {
                        self.write_data(bytes)?;
                    }
                }
                InitOp::DelayMs(ms) => self.delay.delay_ms(u32::from(ms)),
            }
        }
        Ok(())
    }

    /// Set the GRAM address window (inclusive end coordinates).
    pub fn set_window(
        &mut self,
        x0: u16,
        y0: u16,
        x1: u16,
        y1: u16,
    ) -> Result<(), Ili9486Error<SPI::Error, DC::Error>> {
        self.write_cmd(cmd::CASET)?;
        self.write_data(&address_window_bytes(x0, x1))?;
        self.write_cmd(cmd::PASET)?;
        self.write_data(&address_window_bytes(y0, y1))?;
        Ok(())
    }

    /// Fill the full panel with a solid colour.
    ///
    /// Builds one RGB565 frame buffer and clocks it out in a **single** SPI
    /// write so CS stays low for the entire RAMWR payload (see module docs).
    pub fn fill_screen(
        &mut self,
        color: Rgb565,
    ) -> Result<(), Ili9486Error<SPI::Error, DC::Error>> {
        let x1 = self.width.saturating_sub(1);
        let y1 = self.height.saturating_sub(1);
        self.set_window(0, 0, x1, y1)?;
        self.write_cmd(cmd::RAMWR)?;

        let pixel = color.to_be_bytes();
        let total = display::frame_pixels(self.width, self.height) as usize;
        let mut frame = vec![0u8; total.saturating_mul(2)];
        for slot in frame.chunks_exact_mut(2) {
            slot.copy_from_slice(&pixel);
        }

        // One SpiDevice::write ⇒ one CS assertion around the whole buffer.
        self.dc_data()?;
        self.spi.write(&frame).map_err(Ili9486Error::Spi)?;
        Ok(())
    }

    fn write_cmd(&mut self, c: u8) -> Result<(), Ili9486Error<SPI::Error, DC::Error>> {
        self.dc.set_low().map_err(Ili9486Error::Pin)?;
        self.spi.write(&[c]).map_err(Ili9486Error::Spi)
    }

    fn write_data(&mut self, bytes: &[u8]) -> Result<(), Ili9486Error<SPI::Error, DC::Error>> {
        self.dc_data()?;
        self.spi.write(bytes).map_err(Ili9486Error::Spi)
    }

    fn dc_data(&mut self) -> Result<(), Ili9486Error<SPI::Error, DC::Error>> {
        self.dc.set_high().map_err(Ili9486Error::Pin)
    }
}
