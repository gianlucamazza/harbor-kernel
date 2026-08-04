//! ILI9486 TFT panel over SPI (MIPI DBI-style command/data).
//!
//! Board-agnostic panel protocol on top of:
//! - short ops via [`SpiDevice`] (init registers);
//! - long streams via [`ExclusiveDevice::with_bus`] (ADR-0010 RAMWR session).
//!
//! Init tables are declarative [`InitOp`] lists — datasheet opcodes with the
//! Linux `fb_ili9486` PiScreen table as a documented cross-check (ADR-0009).
//!
//! Pixel path: RGB565, big-endian on the wire, 8-bit SPI words. Solid fill
//! **streams** a small stack pattern under one CS — no full-frame heap buffer.

use core::convert::Infallible;

use kernel_core::display::{self, InitOp, Rgb565, address_window_bytes, cmd, madctl};

use crate::drivers::delay::DelayNs;
use crate::drivers::pin::OutputPin;
use crate::drivers::spi::{ExclusiveDevice, ExclusiveDeviceError, SpiBus, SpiDevice};

/// Landscape 480×320 as used by Waveshare-class 3.5″ HATs after MADCTL MV.
pub const WIDTH: u16 = 480;
pub const HEIGHT: u16 = 320;

/// PiScreen / Waveshare-class power and gamma (Linux `fb_ili9486` default).
///
/// Command bytes are ILI9486 / MIPI DCS; parameter blobs match the open fbtft
/// table used on those HATs (cross-check, not an opaque vendor blob).
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
    // Landscape, BGR (fbtft rotate=90 + bgr).
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

type SpiErr<BUS, CS> = ExclusiveDeviceError<<BUS as SpiBus>::Error, <CS as OutputPin>::Error>;

/// ILI9486 on software-CS SPI ([`ExclusiveDevice`]) + DC + delay.
///
/// Bound to [`ExclusiveDevice`] so RAMWR can use [`ExclusiveDevice::with_bus`]
/// (ADR-0010). Short init ops still go through [`SpiDevice`].
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

    /// Split back into bus device and pins (BSP stores the parts after bring-up).
    pub fn into_parts(self) -> (ExclusiveDevice<BUS, CS, DELAY>, DC, D) {
        (self.spi, self.dc, self.delay)
    }

    /// Hardware reset via RST, then run `init` (typically [`INIT_PISCREEN`]).
    pub fn reset_and_init<RST: OutputPin>(
        &mut self,
        rst: &mut RST,
        init: &[InitOp],
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, DC::Error>> {
        let _ = rst.set_low();
        self.delay.delay_ms(20);
        let _ = rst.set_high();
        self.delay.delay_ms(120);
        self.run_init(init)
    }

    /// Execute a declarative init table (short CS transactions per op).
    pub fn run_init(
        &mut self,
        init: &[InitOp],
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, DC::Error>> {
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
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, DC::Error>> {
        self.write_cmd(cmd::CASET)?;
        self.write_data(&address_window_bytes(x0, x1))?;
        self.write_cmd(cmd::PASET)?;
        self.write_data(&address_window_bytes(y0, y1))?;
        Ok(())
    }

    fn write_cmd(
        &mut self,
        c: u8,
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, DC::Error>> {
        self.dc.set_low().map_err(Ili9486Error::Pin)?;
        self.spi.write(&[c]).map_err(Ili9486Error::Spi)
    }

    fn write_data(
        &mut self,
        bytes: &[u8],
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, DC::Error>> {
        self.dc.set_high().map_err(Ili9486Error::Pin)?;
        self.spi.write(bytes).map_err(Ili9486Error::Spi)
    }
}

/// Solid-fill path: DC pin errors are [`Infallible`] so DC can toggle inside a
/// bus session without polluting [`SpiBus::Error`] (ADR-0010). BSP GPIO outputs
/// satisfy this; fallible DC would need a richer session error type later.
impl<BUS, CS, DELAY, DC, D> Ili9486<BUS, CS, DELAY, DC, D>
where
    BUS: SpiBus,
    CS: OutputPin,
    DELAY: DelayNs,
    DC: OutputPin<Error = Infallible>,
    D: DelayNs,
{
    /// Fill the full panel with a solid colour.
    ///
    /// One CS session (ADR-0010): RAMWR command, then repeated FIFO-sized
    /// colour chunks from a **stack** buffer — no full-frame heap allocation.
    pub fn fill_screen(
        &mut self,
        color: Rgb565,
    ) -> Result<(), Ili9486Error<SpiErr<BUS, CS>, Infallible>> {
        let x1 = self.width.saturating_sub(1);
        let y1 = self.height.saturating_sub(1);
        self.set_window(0, 0, x1, y1)?;

        let pixel = color.to_be_bytes();
        // 32 pixels × 2 B = 64 B — matches BCM SPI0 FIFO depth.
        const PIXELS: usize = 32;
        let mut chunk = [0u8; PIXELS * 2];
        for slot in chunk.chunks_exact_mut(2) {
            slot.copy_from_slice(&pixel);
        }

        let total = display::frame_pixels(self.width, self.height) as usize;
        let mut remaining = total;

        let spi = &mut self.spi;
        let dc = &mut self.dc;

        spi.with_bus(|bus| {
            // DBI: DC low → opcode; DC high → RAMWR payload. CS stays low.
            match dc.set_low() {
                Ok(()) => {}
                Err(e) => match e {},
            }
            bus.write(&[cmd::RAMWR])?;
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
}
