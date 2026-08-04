//! BSP bind for the optional SPI status surface (ADR-0009).
//!
//! Owns pinmux, SPI0, control pins, and ILI9486 bring-up for Waveshare-class
//! 3.5″ glass. The resident handle stays available for a later status surface.

use kernel_core::display::Rgb565;
use kernel_core::spi::{self as spi_math, ClockDivError};

use crate::arch::cpu;
use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::{gpio, memmap};
use crate::drivers::delay::{ArchTimerDelay, DelayNs};
use crate::drivers::ili9486::{self, Ili9486, Ili9486Error, INIT_PISCREEN};
use crate::drivers::pin::OutputPin;
use crate::drivers::spi::{
    BcmSpi, BcmSpiError, ExclusiveDevice, ExclusiveDeviceError, SpiDevice,
};
use crate::sync::SyncCell;

/// Waveshare-class LCD chip-select (BCM GPIO 8, header pin 24).
pub const LCD_CS_PIN: u8 = 8;
/// Data/command select (BCM GPIO 24, header pin 18).
pub const LCD_DC_PIN: u8 = 24;
/// Panel reset (BCM GPIO 25, header pin 22).
pub const LCD_RST_PIN: u8 = 25;

/// Why the SPI / panel path could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplaySpiError {
    /// Core/target clock pair is not programmable on SPI0.
    Clock(ClockDivError),
    /// GPIO pinmux or output claim failed.
    Gpio(gpio::GpioError),
    /// Polled SPI transfer timed out or rejected its arguments.
    Bus(ExclusiveDeviceError<BcmSpiError, core::convert::Infallible>),
    /// ILI9486 command stream or fill failed.
    Panel(Ili9486Error<ExclusiveDeviceError<BcmSpiError, core::convert::Infallible>, core::convert::Infallible>),
}

/// SPI0 + software CS + control pins + post-init panel state.
pub struct DisplaySpi {
    device: ExclusiveDevice<BcmSpi, gpio::Output, ArchTimerDelay>,
    dc: gpio::Output,
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

    /// Hardware reset, PiScreen-class init, full-screen solid fill.
    ///
    /// Leaves the panel on with `color` in GRAM — replaces the default white
    /// field of an unprogrammed ILI9486.
    pub fn bringup_panel(&mut self, color: Rgb565) -> Result<(), DisplaySpiError> {
        let mut panel = Ili9486::new(
            &mut self.device,
            &mut self.dc,
            ArchTimerDelay,
            ili9486::WIDTH,
            ili9486::HEIGHT,
        );
        panel
            .reset_and_init(&mut self.rst, INIT_PISCREEN)
            .map_err(DisplaySpiError::Panel)?;
        panel
            .fill_screen(color)
            .map_err(DisplaySpiError::Panel)?;
        Ok(())
    }
}

/// Resident handle after a successful [`init_and_panel`].
static DISPLAY: SyncCell<Option<DisplaySpi>> = SyncCell::new(None);

/// Pinmux SPI0, claim pins, self-test the controller, then program ILI9486 and
/// fill the screen so the glass is not left white.
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

        let cs = gpio
            .claim_output(LCD_CS_PIN, gpio::Pull::None)
            .map_err(DisplaySpiError::Gpio)?;
        let mut dc = gpio
            .claim_output(LCD_DC_PIN, gpio::Pull::None)
            .map_err(DisplaySpiError::Gpio)?;
        let mut rst = gpio
            .claim_output(LCD_RST_PIN, gpio::Pull::None)
            .map_err(DisplaySpiError::Gpio)?;

        let mut delay = ArchTimerDelay;

        // Brief bus self-test while the panel is held in reset.
        let _ = rst.set_low();
        let _ = dc.set_low();
        delay.delay_us(20);

        let bus = BcmSpi::init(Mmio::new(memmap::SPI0_BASE), cdiv);
        let mut device = match ExclusiveDevice::new(bus, cs, ArchTimerDelay, 0) {
            Ok(dev) => dev,
            Err(infallible) => match infallible {},
        };

        selftest_spi_device(&mut device).map_err(DisplaySpiError::Bus)?;

        let mut display = DisplaySpi {
            device,
            dc,
            rst,
            cdiv,
            bit_hz,
        };

        // Full ILI path: HW reset, init table, solid fill (not white).
        display.bringup_panel(Rgb565::HARBOR)?;

        Ok(display)
    }
}

fn selftest_spi_device<D: SpiDevice>(dev: &mut D) -> Result<(), D::Error> {
    dev.write(&[0x00])?;
    let mut read = [0u8; 1];
    dev.transfer(&mut read, &[0x00])?;
    let mut words = [0u8; 1];
    dev.transfer_in_place(&mut words)?;
    Ok(())
}

/// Install the exclusive display stack for later status-surface work.
pub fn install(spi: DisplaySpi) {
    cpu::without_irqs(|| {
        // SAFETY: single core; IRQs masked; voluntary path only.
        unsafe {
            *DISPLAY.get() = Some(spi);
        }
    });
}

/// Run `f` with the installed display stack, IRQs masked for the duration.
pub fn with_display<R>(f: impl FnOnce(&mut DisplaySpi) -> R) -> Option<R> {
    cpu::without_irqs(|| {
        // SAFETY: single core; IRQs masked.
        unsafe { (*DISPLAY.get()).as_mut().map(f) }
    })
}
