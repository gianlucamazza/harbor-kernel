//! BSP bind for the optional SPI status surface (ADR-0009).
//!
//! Owns pinmux, SPI0 construction, and the resident [`DisplaySpi`] handle.
//! Panel protocol (ILI9486) is a separate driver; this module does not speak
//! ILI commands. Accessors for the bus and control pins are added when that
//! driver consumes them — not as unused placeholders.

use kernel_core::spi::{self as spi_math, ClockDivError};

use crate::arch::cpu;
use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::{gpio, memmap};
use crate::drivers::delay::{ArchTimerDelay, DelayNs};
use crate::drivers::pin::OutputPin;
use crate::drivers::spi::{BcmSpi, BcmSpiError, ExclusiveDevice, ExclusiveDeviceError, SpiDevice};
use crate::sync::SyncCell;

/// Waveshare-class LCD chip-select (BCM GPIO 8, header pin 24).
pub const LCD_CS_PIN: u8 = 8;
/// Data/command select (BCM GPIO 24, header pin 18).
pub const LCD_DC_PIN: u8 = 24;
/// Panel reset (BCM GPIO 25, header pin 22).
pub const LCD_RST_PIN: u8 = 25;

/// Why the SPI stack for the display path could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplaySpiError {
    /// Core/target clock pair is not programmable on SPI0.
    Clock(ClockDivError),
    /// GPIO pinmux or output claim failed.
    Gpio(gpio::GpioError),
    /// Polled SPI transfer timed out or rejected its arguments.
    Bus(ExclusiveDeviceError<BcmSpiError, core::convert::Infallible>),
}

/// SPI0 + software CS + control pins for the LCD.
///
/// Ready for an ILI9486 driver. The bus and pin fields are the resident
/// resource; diagnostics use [`DisplaySpi::cdiv`] / [`DisplaySpi::bit_hz`].
/// Field readers land with the panel driver — until then the handle exists to
/// own the hardware, not to be torn down after a log line.
pub struct DisplaySpi {
    // First consumer: ILI9486 init (command stream + DC/RST sequencing).
    #[allow(dead_code)]
    device: ExclusiveDevice<BcmSpi, gpio::Output, ArchTimerDelay>,
    #[allow(dead_code)]
    dc: gpio::Output,
    #[allow(dead_code)]
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
}

/// Resident handle after a successful [`init_spi`] + [`install`].
///
/// Single-core, voluntary path only (ADR-0009: IRQ never paints). Mutated under
/// [`cpu::without_irqs`].
static DISPLAY: SyncCell<Option<DisplaySpi>> = SyncCell::new(None);

/// Pinmux SPI0, claim CS/DC/RST, init the controller, and self-test the SPI
/// path with the panel **held in reset** so glass state is not programmed.
///
/// On success CS is idle-high and reset is released (high) for the panel
/// driver to sequence properly.
///
/// # Safety
///
/// Exclusive ownership of GPIO and SPI0 MMIO. Call after the UART pinmux so
/// both share the same early-boot exclusivity window (single core, IRQs still
/// under bootstrap control).
pub unsafe fn init_spi() -> Result<DisplaySpi, DisplaySpiError> {
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

        // Hold the panel in reset for the controller self-test so SPI activity
        // cannot be mistaken for an intentional command stream.
        let _ = rst.set_low();
        let _ = dc.set_low();
        delay.delay_us(20);

        let bus = BcmSpi::init(Mmio::new(memmap::SPI0_BASE), cdiv);
        // cs_idle_ns = 0 until the panel datasheet requires a hold time.
        let mut device = match ExclusiveDevice::new(bus, cs, delay, 0) {
            Ok(dev) => dev,
            Err(infallible) => match infallible {},
        };

        selftest_spi_device(&mut device).map_err(DisplaySpiError::Bus)?;

        // Release reset so a later panel driver starts from a known idle line.
        let _ = rst.set_high();
        // `device` owns the other ArchTimerDelay; use a fresh one for settle.
        ArchTimerDelay.delay_us(20);

        Ok(DisplaySpi {
            device,
            dc,
            rst,
            cdiv,
            bit_hz,
        })
    }
}

/// Full-duplex paths under CS while the panel is held in reset.
fn selftest_spi_device<D: SpiDevice>(dev: &mut D) -> Result<(), D::Error> {
    dev.write(&[0x00])?;
    let mut read = [0u8; 1];
    dev.transfer(&mut read, &[0x00])?;
    let mut words = [0u8; 1];
    dev.transfer_in_place(&mut words)?;
    Ok(())
}

/// Install the exclusive display SPI stack for later panel/status use.
pub fn install(spi: DisplaySpi) {
    cpu::without_irqs(|| {
        // SAFETY: single core; IRQs masked; voluntary path only.
        unsafe {
            *DISPLAY.get() = Some(spi);
        }
    });
}

/// Run `f` with the installed display stack, IRQs masked for the duration.
///
/// `None` if [`install`] has not run.
pub fn with_display<R>(f: impl FnOnce(&mut DisplaySpi) -> R) -> Option<R> {
    cpu::without_irqs(|| {
        // SAFETY: single core; IRQs masked.
        unsafe { (*DISPLAY.get()).as_mut().map(f) }
    })
}
