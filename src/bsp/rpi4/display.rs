//! BSP bind for the optional SPI status surface (ADR-0009 / ADR-0010).
//!
//! Owns pinmux, SPI0, control pins, and ILI9486 bring-up for Waveshare-class
//! 3.5″ glass. The resident handle stays available for a later status surface.

use kernel_core::display::Rgb565;
use kernel_core::spi::{self as spi_math, ClockDivError};

use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::{gpio, memmap};
use crate::drivers::delay::{ArchTimerDelay, DelayNs};
use crate::drivers::ili9486::{self, INIT_PISCREEN, Ili9486, Ili9486Error};
use crate::drivers::pin::OutputPin;
use crate::drivers::spi::{BcmSpi, BcmSpiError, ExclusiveDevice, ExclusiveDeviceError, SpiBus};
use crate::sync::Mutex;

/// Waveshare-class LCD chip-select (BCM GPIO 8, header pin 24).
pub const LCD_CS_PIN: u8 = 8;
/// Data/command select (BCM GPIO 24, header pin 18).
pub const LCD_DC_PIN: u8 = 24;
/// Panel reset (BCM GPIO 25, header pin 22).
pub const LCD_RST_PIN: u8 = 25;

type Dev = ExclusiveDevice<BcmSpi, gpio::Output, ArchTimerDelay>;
type PanelErr = Ili9486Error<
    ExclusiveDeviceError<BcmSpiError, core::convert::Infallible>,
    core::convert::Infallible,
>;

/// Why the SPI / panel path could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplaySpiError {
    /// Core/target clock pair is not programmable on SPI0.
    Clock(ClockDivError),
    /// GPIO pinmux or output claim failed.
    Gpio(gpio::GpioError),
    /// SPI0 controller transfer failed (pre-panel smoke).
    Bus(BcmSpiError),
    /// ILI9486 command stream or fill failed.
    Panel(PanelErr),
}

/// SPI0 + software CS + control pins after panel bring-up.
///
/// `device` / `dc` live in [`Option`] so [`Self::with_panel`] can **take** them
/// into a transient [`Ili9486`] and put them back — no `ptr::read` on half-moved
/// fields (architecture ownership honesty).
pub struct DisplaySpi {
    device: Option<Dev>,
    dc: Option<gpio::Output>,
    /// Held after reset so a later soft re-init can pulse without re-claim.
    #[expect(
        dead_code,
        reason = "held to keep the panel out of reset for the board's life"
    )]
    rst: gpio::Output,
    cdiv: u32,
    bit_hz: u32,
}

impl DisplaySpi {
    /// Programmed clock divisor encoding.
    #[inline]
    pub fn cdiv(&self) -> u32 {
        self.cdiv
    }

    /// Effective SPI bit clock (Hz).
    #[inline]
    pub fn bit_hz(&self) -> u32 {
        self.bit_hz
    }

    /// Run `f` with a transient [`Ili9486`] view of the resident bus and DC pin.
    ///
    /// Takes ownership of the SPI device and DC pin for the duration of `f`,
    /// then restores them. Nested calls or a missing install are a programming
    /// error (panic). Callers keep IRQs masked via [`with_display`].
    pub fn with_panel<R>(
        &mut self,
        f: impl FnOnce(
            &mut Ili9486<BcmSpi, gpio::Output, ArchTimerDelay, gpio::Output, ArchTimerDelay>,
        ) -> R,
    ) -> R {
        let device = self
            .device
            .take()
            .expect("display: SPI device missing (nested with_panel?)");
        let dc = self
            .dc
            .take()
            .expect("display: DC pin missing (nested with_panel?)");
        let mut panel = Ili9486::new(device, dc, ArchTimerDelay, ili9486::WIDTH, ili9486::HEIGHT);
        let result = f(&mut panel);
        let (device, dc, _) = panel.into_parts();
        self.device = Some(device);
        self.dc = Some(dc);
        result
    }
}

/// Resident handle after a successful [`init_and_panel`].
static DISPLAY: Mutex<Option<DisplaySpi>> = Mutex::new(None);

/// Pinmux SPI0, claim pins, program ILI9486, fill status background (HARBOR).
///
/// Product boot path (ADR-0009): PiScreen init + full navy fill. Colour-bar
/// diagnostics live on [`Ili9486::draw_color_bars`] for lab re-proof only.
///
/// # Safety
///
/// Exclusive ownership of GPIO and SPI0 MMIO. Call after the UART pinmux.
pub unsafe fn init_and_panel() -> Result<DisplaySpi, DisplaySpiError> {
    let cdiv = spi_math::clock_divisor(memmap::SPI0_CORE_CLOCK_HZ, memmap::SPI0_TARGET_HZ)
        .map_err(DisplaySpiError::Clock)?;
    let bit_hz = spi_math::effective_hz(memmap::SPI0_CORE_CLOCK_HZ, cdiv);

    // SAFETY: caller holds exclusive GPIO + SPI0.
    unsafe {
        let gpio = gpio::Gpio::new();
        gpio::configure_spi0_data_pins(&gpio);

        let mut cs = gpio
            .claim_output(LCD_CS_PIN, gpio::Pull::None)
            .map_err(DisplaySpiError::Gpio)?;
        let dc = gpio
            .claim_output(LCD_DC_PIN, gpio::Pull::None)
            .map_err(DisplaySpiError::Gpio)?;
        let mut rst = gpio
            .claim_output(LCD_RST_PIN, gpio::Pull::None)
            .map_err(DisplaySpiError::Gpio)?;

        // Quiet the glass until Ili9486 sequences a proper HW reset.
        let _ = rst.set_low();
        let _ = cs.set_high(); // deselect while controller is smoked
        ArchTimerDelay.delay_us(20);

        let mut bus = BcmSpi::init(Mmio::new(memmap::SPI0_BASE), cdiv);
        // Controller smoke with CS GPIO high: MOSI/SCLK move, slave ignores.
        let mut rx = [0u8; 1];
        bus.transfer(&mut rx, &[0x00])
            .map_err(DisplaySpiError::Bus)?;
        let mut words = [0u8; 1];
        bus.transfer_in_place(&mut words)
            .map_err(DisplaySpiError::Bus)?;

        let device = match ExclusiveDevice::new(bus, cs, ArchTimerDelay, 0) {
            Ok(dev) => dev,
            Err(infallible) => match infallible {},
        };

        let mut panel = Ili9486::new(device, dc, ArchTimerDelay, ili9486::WIDTH, ili9486::HEIGHT);
        panel
            .reset_and_init(&mut rst, INIT_PISCREEN)
            .map_err(DisplaySpiError::Panel)?;
        // Full status background. Text is painted by `status` via dirty cells.
        // (Lab colour bars are not part of product boot — use draw_color_bars.)
        panel
            .fill_screen(Rgb565::HARBOR)
            .map_err(DisplaySpiError::Panel)?;

        let (device, dc, _delay) = panel.into_parts();

        Ok(DisplaySpi {
            device: Some(device),
            dc: Some(dc),
            rst,
            cdiv,
            bit_hz,
        })
    }
}

/// Install the exclusive display stack for later status-surface work.
pub fn install(spi: DisplaySpi) {
    DISPLAY.with(|slot| *slot = Some(spi));
}

/// Run `f` with the installed display stack, IRQs masked for the duration.
pub fn with_display<R>(f: impl FnOnce(&mut DisplaySpi) -> R) -> Option<R> {
    DISPLAY.with(|slot| slot.as_mut().map(f))
}
