//! SPI master contracts, software-CS device wrapper, and BCM2711 SPI0.
//!
//! Shape aligned with embedded-hal 1.0: [`SpiBus`] is the controller,
//! [`SpiDevice`] is one slave selected by its own chip-select. Panel and
//! (later) touch drivers take a [`SpiDevice`] so they never toggle CS by hand
//! (ADR-0009).

pub mod bcm;

pub use bcm::{BcmSpi, BcmSpiError};
// `BcmSpiError` is part of the public surface for BSP error mapping.

use crate::drivers::delay::DelayNs;
use crate::drivers::pin::OutputPin;

/// Full-duplex SPI bus (no chip-select).
///
/// Word size is 8-bit. Mode and rate are programmed when the concrete bus is
/// constructed, not on each transfer — matching how the BCM SPI block is used.
pub trait SpiBus {
    /// Failure mode of this bus implementation.
    type Error;

    /// Clock out `words`, discarding MISO.
    fn write(&mut self, words: &[u8]) -> Result<(), Self::Error>;

    /// Clock `words` out while filling `read` from MISO.
    ///
    /// `read` and `words` must be the same length; the implementation returns
    /// an error if they are not.
    fn transfer(&mut self, read: &mut [u8], words: &[u8]) -> Result<(), Self::Error>;

    /// Full-duplex in place: each MISO byte overwrites the MOSI slot.
    fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error>;
}

/// SPI slave with its own chip-select.
///
/// Every method asserts CS for the duration of the transfer and deasserts it
/// afterwards. Drivers that talk to a single device (ILI9486, XPT2046) depend
/// only on this trait.
pub trait SpiDevice {
    /// Failure mode of this device implementation.
    type Error;

    /// Clock out `words`, discarding MISO, under CS.
    fn write(&mut self, words: &[u8]) -> Result<(), Self::Error>;

    /// Full-duplex transfer under CS (`read` and `words` same length).
    fn transfer(&mut self, read: &mut [u8], words: &[u8]) -> Result<(), Self::Error>;

    /// Full-duplex in place under CS.
    fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error>;
}

/// Why [`ExclusiveDevice`] could not complete a transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExclusiveDeviceError<B, P> {
    /// The underlying bus failed.
    Bus(B),
    /// The CS pin failed.
    Pin(P),
}

/// One SPI slave with software (GPIO) chip-select.
///
/// Active-low CS, which is the convention on the Waveshare-class HAT and most
/// SPI panels. A short post-deassert delay absorbs CS-to-idle hold when the
/// slave datasheet requires it; zero is a valid delay.
pub struct ExclusiveDevice<BUS, CS, D> {
    bus: BUS,
    cs: CS,
    delay: D,
    /// Nanoseconds to wait after releasing CS, before the next transaction.
    cs_idle_ns: u32,
}

impl<BUS, CS, D> ExclusiveDevice<BUS, CS, D>
where
    BUS: SpiBus,
    CS: OutputPin,
    D: DelayNs,
{
    /// Build a device with CS held idle (high) after construction.
    ///
    /// # Errors
    ///
    /// Returns the pin error if driving CS high fails.
    pub fn new(bus: BUS, mut cs: CS, delay: D) -> Result<Self, CS::Error> {
        cs.set_high()?;
        Ok(Self {
            bus,
            cs,
            delay,
            cs_idle_ns: 0,
        })
    }

    /// Set the post-deassert idle delay (nanoseconds).
    pub fn with_cs_idle_ns(mut self, ns: u32) -> Self {
        self.cs_idle_ns = ns;
        self
    }

    fn with_cs<R>(
        &mut self,
        body: impl FnOnce(&mut BUS) -> Result<R, BUS::Error>,
    ) -> Result<R, ExclusiveDeviceError<BUS::Error, CS::Error>> {
        self.cs.set_low().map_err(ExclusiveDeviceError::Pin)?;
        let result = body(&mut self.bus).map_err(ExclusiveDeviceError::Bus);
        // Always release CS, even when the transfer failed.
        let pin = self.cs.set_high().map_err(ExclusiveDeviceError::Pin);
        if self.cs_idle_ns != 0 {
            self.delay.delay_ns(self.cs_idle_ns);
        }
        // Prefer reporting a bus error over a later pin error, but never leave
        // CS stuck low because we returned early.
        match (result, pin) {
            (Err(e), _) => Err(e),
            (Ok(_), Err(e)) => Err(e),
            (Ok(v), Ok(())) => Ok(v),
        }
    }
}

impl<BUS, CS, D> SpiDevice for ExclusiveDevice<BUS, CS, D>
where
    BUS: SpiBus,
    CS: OutputPin,
    D: DelayNs,
{
    type Error = ExclusiveDeviceError<BUS::Error, CS::Error>;

    fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        self.with_cs(|bus| bus.write(words))
    }

    fn transfer(&mut self, read: &mut [u8], words: &[u8]) -> Result<(), Self::Error> {
        self.with_cs(|bus| bus.transfer(read, words))
    }

    fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        self.with_cs(|bus| bus.transfer_in_place(words))
    }
}
