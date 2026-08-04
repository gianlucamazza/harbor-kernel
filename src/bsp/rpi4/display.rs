//! BSP bind for the optional SPI status surface (ADR-0009).
//!
//! Pinmux + SPI0 construction and a bus smoke transfer. Panel protocol
//! (ILI9486) is a driver; this module does not speak ILI commands.

use kernel_core::spi::{self as spi_math, ClockDivError};

use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::{gpio, memmap};
use crate::drivers::delay::ArchTimerDelay;
use crate::drivers::pin::OutputPin;
use crate::drivers::spi::{BcmSpi, BcmSpiError, ExclusiveDevice, ExclusiveDeviceError, SpiDevice};

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

/// SPI0 + software CS for the LCD, ready for an ILI9486 driver.
pub struct DisplaySpi {
    pub device: ExclusiveDevice<BcmSpi, gpio::Output, ArchTimerDelay>,
    pub dc: gpio::Output,
    pub rst: gpio::Output,
    /// Programmed CDIV encoding (for diagnostics).
    pub cdiv: u32,
    /// Effective bit clock in Hz (for diagnostics).
    pub bit_hz: u32,
}

/// Pinmux SPI0 data pins, claim CS/DC/RST outputs, init the controller, and
/// run an empty full-duplex smoke transfer (proves TA/FIFO/DONE, not the panel).
///
/// CS is idle-high on success. Does not reset or configure the ILI9486.
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
        gpio::configure_spi0_data_pins();
        let gpio = gpio::Gpio::new();
        let cs = gpio
            .claim_output(LCD_CS_PIN, gpio::Pull::None)
            .map_err(DisplaySpiError::Gpio)?;
        let dc = gpio
            .claim_output(LCD_DC_PIN, gpio::Pull::None)
            .map_err(DisplaySpiError::Gpio)?;
        let mut rst = gpio
            .claim_output(LCD_RST_PIN, gpio::Pull::None)
            .map_err(DisplaySpiError::Gpio)?;
        // Hold reset released (high) until the panel driver sequences it.
        let _ = rst.set_high();

        let bus = BcmSpi::init(Mmio::new(memmap::SPI0_BASE), cdiv);
        // 1 ns post-CS idle exercises the DelayNs path; negligible on the wire.
        let mut device = ExclusiveDevice::new(bus, cs, ArchTimerDelay)
            .expect("Infallible GPIO output")
            .with_cs_idle_ns(1);

        // Empty transfers: start/stop + CS bracket without clocking a slave.
        smoke_bus(&mut device).map_err(DisplaySpiError::Bus)?;

        Ok(DisplaySpi {
            device,
            dc,
            rst,
            cdiv,
            bit_hz,
        })
    }
}

/// Exercise [`SpiDevice`] without depending on panel glass.
fn smoke_bus<D: SpiDevice>(dev: &mut D) -> Result<(), D::Error> {
    dev.write(&[])?;
    let mut read = [];
    dev.transfer(&mut read, &[])?;
    let mut words = [];
    dev.transfer_in_place(&mut words)?;
    Ok(())
}

/// Touch the delay helpers the panel driver will use (ms / us), so the arch
/// timer paths stay linked under `debug-display`.
pub fn smoke_delays() {
    use crate::drivers::delay::DelayNs;
    let mut d = ArchTimerDelay;
    d.delay_us(1);
    d.delay_ms(0);
}
