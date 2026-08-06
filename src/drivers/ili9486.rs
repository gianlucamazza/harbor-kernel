//! ILI9486 TFT panel over SPI (MIPI DBI-style command/data).
//!
//! Board-agnostic panel protocol on top of:
//! - short ops via [`SpiDevice`] (init registers);
//! - long streams via [`ExclusiveDevice::with_bus`] (ADR-0010 RAMWR session).
//!
//! Init is declarative [`InitOp`] — datasheet opcodes; parameters match Linux
//! `fb_ili9486` PiScreen under fbtft.
//!
//! **Wire framing (Waveshare / PiScreen SKU):** fbtft `piscreen` uses
//! `regwidth=16` + `buswidth=8`. Every command and every parameter byte is
//! sent as a big-endian `u16` with high byte zero (`0x00, opcode`). Pixel
//! RGB565 words are raw 16-bit colour (not zero-padded). Sending bare 8-bit
//! commands on that interface yields noise / faint lines on glass.
//!
//! Pixel path: RGB565 big-endian, COLMOD `0x55`. Solid fills stream under one CS.

use core::convert::Infallible;

use kernel_core::display::{
    InitOp, Rgb565, address_window_bytes, cmd, expand_reg16_be, madctl, reg16_be,
};

use crate::drivers::delay::DelayNs;
use crate::drivers::pin::OutputPin;
use crate::drivers::spi::{ExclusiveDevice, ExclusiveDeviceError, SpiBus};

/// Logical landscape size after MADCTL row/column exchange (Waveshare HAT view).
pub const WIDTH: u16 = 480;
pub const HEIGHT: u16 = 320;

/// PiScreen / Waveshare-class power and gamma (Linux `fb_ili9486` default).
///
/// Matches the open fbtft table used on those HATs (cross-check, not a blob),
/// plus MADCTL for landscape + BGR (fbtft rotate=90 + bgr → `0x28`).
///
/// Deliberately **no** experimental C0/C1 / SWRESET / INVOFF extras: those
/// washed the glass gray on silicon while this table + solid fill produced the
/// earlier azure proof (with the CS-session fix from ADR-0010).
pub const INIT_PISCREEN: &[InitOp] = &[
    InitOp::Cmd(cmd::IFMODE),
    InitOp::Data(&[0x00]),
    InitOp::Cmd(cmd::SLPOUT),
    InitOp::DelayMs(250), // fbtft PiScreen
    InitOp::Cmd(cmd::COLMOD),
    InitOp::Data(&[0x55]), // 16 bpp RGB565
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
    // Landscape + BGR (fbtft rotate 90 + bgr → 0x20|0x08).
    InitOp::Cmd(cmd::MADCTL),
    InitOp::Data(&[madctl::MV | madctl::BGR]),
    InitOp::Cmd(cmd::DISPON),
    InitOp::DelayMs(20),
];

/// Why a panel operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ili9486Error<S, P> {
    Spi(S),
    #[expect(dead_code, reason = "reserved for non-Infallible DC pins")]
    Pin(P),
}

type SpiErr<BUS, CS> = ExclusiveDeviceError<<BUS as SpiBus>::Error, <CS as OutputPin>::Error>;

/// ILI9486 on software-CS SPI ([`ExclusiveDevice`]) + DC + delay.
pub struct Ili9486<BUS, CS, DELAY, DC, D> {
    spi: ExclusiveDevice<BUS, CS, DELAY>,
    dc: DC,
    delay: D,
    width: u16,
    height: u16,
}

impl<BUS, CS, DELAY, DC, D> Ili9486<BUS, CS, DELAY, DC, D>
where
    BUS: SpiBus,
    CS: OutputPin,
    DELAY: DelayNs,
    DC: OutputPin,
    D: DelayNs,
{
    /// Bind the panel without programming it.
    pub fn new(
        spi: ExclusiveDevice<BUS, CS, DELAY>,
        dc: DC,
        delay: D,
        width: u16,
        height: u16,
    ) -> Self {
        Self {
            spi,
            dc,
            delay,
            width,
            height,
        }
    }

    /// Split back into bus device and pins.
    pub fn into_parts(self) -> (ExclusiveDevice<BUS, CS, DELAY>, DC, D) {
        (self.spi, self.dc, self.delay)
    }

    /// Hardware reset via RST, then run `init`.
    pub fn reset_and_init<RST: OutputPin>(
        &mut self,
        rst: &mut RST,
        init: &[InitOp],
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>>
    where
        DC: OutputPin<Error = Infallible>,
    {
        // Datasheet-class reset pulse.
        let _ = rst.set_low();
        self.delay.delay_ms(20);
        let _ = rst.set_high();
        self.delay.delay_ms(150);
        self.run_init(init)
    }

    /// Execute a declarative init table.
    ///
    /// A `Cmd` followed by `Data` is issued under **one** CS assertion (DBI
    /// register write). Lone commands and delays stay separate.
    pub fn run_init(
        &mut self,
        init: &[InitOp],
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, DC::Error>>
    where
        DC: OutputPin<Error = Infallible>,
    {
        let mut i = 0;
        while i < init.len() {
            match init[i] {
                InitOp::Cmd(c) => {
                    let data = match init.get(i + 1) {
                        Some(InitOp::Data(bytes)) => {
                            i += 1;
                            *bytes
                        }
                        _ => &[][..],
                    };
                    self.write_register(c, data)?;
                }
                InitOp::Data(_) => {
                    // Orphan data without a preceding cmd — skip (table bug).
                }
                InitOp::DelayMs(ms) => self.delay.delay_ms(u32::from(ms)),
            }
            i += 1;
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
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>>
    where
        DC: OutputPin<Error = Infallible>,
    {
        self.write_register(cmd::CASET, &address_window_bytes(x0, x1))?;
        self.write_register(cmd::PASET, &address_window_bytes(y0, y1))?;
        Ok(())
    }

    /// Command + optional parameters under one CS (Infallible DC).
    ///
    /// Waveshare/PiScreen: each logical byte is a BE `u16` on the wire
    /// ([`reg16_be`] / [`expand_reg16_be`]).
    fn write_register(
        &mut self,
        c: u8,
        data: &[u8],
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>>
    where
        DC: OutputPin<Error = Infallible>,
    {
        let spi = &mut self.spi;
        let dc = &mut self.dc;
        // Gamma tables are 15 bytes → 30 wire bytes; keep headroom.
        const MAX_PARAM: usize = 16;
        let mut wire = [0u8; MAX_PARAM * 2];
        spi.with_bus(|bus| {
            match dc.set_low() {
                Ok(()) => {}
                Err(e) => match e {},
            }
            bus.write(&reg16_be(c))?;
            if !data.is_empty() {
                match dc.set_high() {
                    Ok(()) => {}
                    Err(e) => match e {},
                }
                // Stream params in MAX_PARAM-sized logical chunks if ever larger.
                let mut off = 0;
                while off < data.len() {
                    let end = (off + MAX_PARAM).min(data.len());
                    let n = expand_reg16_be(&data[off..end], &mut wire);
                    bus.write(&wire[..n])?;
                    off = end;
                }
            }
            Ok(())
        })
        .map_err(Ili9486Error::Spi)
    }
}

impl<BUS, CS, DELAY, DC, D> Ili9486<BUS, CS, DELAY, DC, D>
where
    BUS: SpiBus,
    CS: OutputPin,
    DELAY: DelayNs,
    DC: OutputPin<Error = Infallible>,
    D: DelayNs,
{
    /// Fill the full logical panel with a solid colour (streaming, one CS).
    pub fn fill_screen(
        &mut self,
        color: Rgb565,
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>> {
        let x1 = self.width.saturating_sub(1);
        let y1 = self.height.saturating_sub(1);
        self.fill_rect(0, 0, x1, y1, color)
    }

    /// Fill an inclusive rectangle (streaming, one CS).
    pub fn fill_rect(
        &mut self,
        x0: u16,
        y0: u16,
        x1: u16,
        y1: u16,
        color: Rgb565,
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>> {
        if x1 < x0 || y1 < y0 {
            return Ok(());
        }
        self.set_window(x0, y0, x1, y1)?;
        let total = (x1 as usize - x0 as usize + 1) * (y1 as usize - y0 as usize + 1);
        self.ramwr_solid(total, color)
    }

    /// Five full-width colour bars (R,G,B,W,Black) — lab / self-test only.
    ///
    /// Not called from product boot (ADR-0009 status surface uses navy fill +
    /// text). Kept to re-proof SPI framing on glass without a full framebuffer.
    #[expect(dead_code, reason = "lab self-test; re-proofs SPI framing on glass")]
    pub fn draw_color_bars(&mut self) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>> {
        let colors = [
            Rgb565::RED,
            Rgb565::GREEN,
            Rgb565::BLUE,
            Rgb565::WHITE,
            Rgb565::BLACK,
        ];
        let h = self.height;
        let w = self.width;
        let band = (h / colors.len() as u16).max(1);
        for (i, &c) in colors.iter().enumerate() {
            let y0 = i as u16 * band;
            let y1 = if i + 1 == colors.len() {
                h.saturating_sub(1)
            } else {
                (y0 + band).saturating_sub(1).min(h.saturating_sub(1))
            };
            self.fill_rect(0, y0, w.saturating_sub(1), y1, c)?;
        }
        Ok(())
    }

    /// Blit a pre-rasterised RGB565 buffer (big-endian pairs).
    pub fn blit_rgb565(
        &mut self,
        x0: u16,
        y0: u16,
        width: u16,
        height: u16,
        pixels: &[u8],
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>> {
        let need = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(2);
        if pixels.len() < need || width == 0 || height == 0 {
            return Ok(());
        }
        let x1 = x0.saturating_add(width - 1);
        let y1 = y0.saturating_add(height - 1);
        self.set_window(x0, y0, x1, y1)?;
        self.ramwr_bytes(&pixels[..need])
    }

    fn ramwr_solid(
        &mut self,
        pixel_count: usize,
        color: Rgb565,
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>> {
        let pixel = color.to_be_bytes();
        const PIXELS: usize = 32;
        let mut chunk = [0u8; PIXELS * 2];
        for slot in chunk.chunks_exact_mut(2) {
            slot.copy_from_slice(&pixel);
        }
        let mut remaining = pixel_count;
        let spi = &mut self.spi;
        let dc = &mut self.dc;
        spi.with_bus(|bus| {
            match dc.set_low() {
                Ok(()) => {}
                Err(e) => match e {},
            }
            // Opcode framed reg16; payload is raw RGB565 (not zero-padded).
            bus.write(&reg16_be(cmd::RAMWR))?;
            match dc.set_high() {
                Ok(()) => {}
                Err(e) => match e {},
            }
            while remaining > 0 {
                let n = remaining.min(PIXELS);
                bus.write(&chunk[..n * 2])?;
                remaining -= n;
            }
            Ok(())
        })
        .map_err(Ili9486Error::Spi)
    }

    fn ramwr_bytes(
        &mut self,
        data: &[u8],
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>> {
        let spi = &mut self.spi;
        let dc = &mut self.dc;
        spi.with_bus(|bus| {
            match dc.set_low() {
                Ok(()) => {}
                Err(e) => match e {},
            }
            bus.write(&reg16_be(cmd::RAMWR))?;
            match dc.set_high() {
                Ok(()) => {}
                Err(e) => match e {},
            }
            const CHUNK: usize = 64;
            let mut off = 0;
            while off < data.len() {
                let end = (off + CHUNK).min(data.len());
                bus.write(&data[off..end])?;
                off = end;
            }
            Ok(())
        })
        .map_err(Ili9486Error::Spi)
    }
}
