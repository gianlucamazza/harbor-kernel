//! BCM2711 GPIO controller — pinmux, pull, and output drive.
//!
//! Board-specific register layout. Drivers depend on
//! [`crate::drivers::pin::OutputPin`], not on this module.
//!
//! Reference: BCM2711 ARM Peripherals (GPFSEL / GPSET / GPCLR / GPPUPPDN).

use crate::arch::mmio::Mmio;
use crate::arch::timer;
use crate::bsp::rpi4::memmap::GPIO_BASE;

/// Highest GPIO index this module will touch (BCM2711 exposes 0–57).
pub const PIN_MAX: u8 = 57;

const GPFSEL0: usize = 0x00;
#[cfg(feature = "debug-display")]
const GPSET0: usize = 0x1C;
#[cfg(feature = "debug-display")]
const GPCLR0: usize = 0x28;
const GPPUPPDN0: usize = 0xE4;

/// Pin function select (3-bit field in GPFSELn).
///
/// Only the encodings in use are listed; add others when a peripheral needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Function {
    /// Push-pull output (CS, DC, RST, …).
    #[cfg(feature = "debug-display")]
    Output = 0b001,
    /// SPI0 data/clock, PL011 UART0 TX/RX on the usual pins, etc.
    Alt0 = 0b100,
}

/// Pad pull on BCM2711 (`GPPUPPDNn`, 2 bits per pin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Pull {
    None = 0b00,
}

/// Why a GPIO operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpioError {
    /// Pin index is outside the supported range.
    InvalidPin,
}

/// Exclusive handle for the GPIO register block.
///
/// Constructed once during board bring-up while a single core owns the
/// controller. Multiple [`Output`] pins may be derived from it; they share the
/// MMIO base by value (a physical address), not a runtime borrow.
pub struct Gpio {
    regs: Mmio,
}

impl Gpio {
    /// Bind the BCM2711 GPIO controller.
    ///
    /// # Safety
    ///
    /// Caller must hold exclusive ownership of the GPIO MMIO window for the
    /// lifetime of this handle and of every [`Output`] derived from it.
    #[inline]
    pub unsafe fn new() -> Self {
        // SAFETY: caller guarantees exclusive ownership of `GPIO_BASE`.
        Self {
            regs: unsafe { Mmio::new(GPIO_BASE) },
        }
    }

    /// Program the function select for `pin`.
    pub fn set_function(&self, pin: u8, function: Function) -> Result<(), GpioError> {
        check_pin(pin)?;
        let reg = GPFSEL0 + (pin as usize / 10) * 4;
        let shift = (u32::from(pin) % 10) * 3;
        let mut val = self.regs.read32(reg);
        val &= !(0b111 << shift);
        val |= u32::from(function as u8) << shift;
        self.regs.write32(reg, val);
        Ok(())
    }

    /// Program the pad pull for `pin`.
    pub fn set_pull(&self, pin: u8, pull: Pull) -> Result<(), GpioError> {
        check_pin(pin)?;
        let reg = GPPUPPDN0 + (pin as usize / 16) * 4;
        let shift = (u32::from(pin) % 16) * 2;
        let mut val = self.regs.read32(reg);
        val &= !(0b11 << shift);
        val |= u32::from(pull as u8) << shift;
        self.regs.write32(reg, val);
        Ok(())
    }

    /// Configure `pin` as a push-pull output and return a drive handle.
    #[cfg(feature = "debug-display")]
    pub fn claim_output(&self, pin: u8, pull: Pull) -> Result<Output, GpioError> {
        self.set_function(pin, Function::Output)?;
        self.set_pull(pin, pull)?;
        Ok(Output {
            regs: self.regs,
            pin,
        })
    }

    /// Configure `pin` for an alternate function (SPI, UART, …).
    pub fn configure_alt(&self, pin: u8, function: Function, pull: Pull) -> Result<(), GpioError> {
        match function {
            Function::Alt0 => {}
            #[cfg(feature = "debug-display")]
            Function::Output => return Err(GpioError::InvalidPin),
        }
        self.set_function(pin, function)?;
        self.set_pull(pin, pull)?;
        Ok(())
    }
}

/// One GPIO line configured as an output.
#[cfg(feature = "debug-display")]
#[derive(Clone, Copy)]
pub struct Output {
    regs: Mmio,
    pin: u8,
}

#[cfg(feature = "debug-display")]
impl Output {
    fn set_level(self, high: bool) {
        let (reg_base, bit) = if self.pin < 32 {
            (if high { GPSET0 } else { GPCLR0 }, self.pin)
        } else {
            (if high { GPSET0 + 4 } else { GPCLR0 + 4 }, self.pin - 32)
        };
        self.regs.write32(reg_base, 1u32 << bit);
    }
}

#[cfg(feature = "debug-display")]
impl crate::drivers::pin::OutputPin for Output {
    type Error = core::convert::Infallible;

    #[inline]
    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.set_level(true);
        Ok(())
    }

    #[inline]
    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.set_level(false);
        Ok(())
    }
}

#[inline]
fn check_pin(pin: u8) -> Result<(), GpioError> {
    if pin > PIN_MAX {
        Err(GpioError::InvalidPin)
    } else {
        Ok(())
    }
}

/// Configure GPIO 14/15 for PL011 UART0 and disable pad pull.
///
/// # Safety
///
/// Must run with exclusive ownership of the GPIO controller (true at early
/// boot on a single active core before any other driver touches GPIO).
pub unsafe fn configure_uart0_pins() {
    // SAFETY: caller holds exclusive GPIO ownership at early boot.
    let gpio = unsafe { Gpio::new() };
    // UART0 is ALT0 on 14/15; pins are in range by construction.
    let _ = gpio.configure_alt(14, Function::Alt0, Pull::None);
    let _ = gpio.configure_alt(15, Function::Alt0, Pull::None);
    // Wall-time settle (~1 µs) rather than a CPU-cycle guess.
    timer::busy_wait_us(1);
}

/// Pinmux SPI0 data/clock lines on an existing [`Gpio`] claim.
///
/// GPIO 9 = MISO, 10 = MOSI, 11 = SCLK — all ALT0. Does not touch CE0/CE1
/// (software CS via [`crate::drivers::spi::ExclusiveDevice`]).
#[cfg(feature = "debug-display")]
pub fn configure_spi0_data_pins(gpio: &Gpio) {
    for pin in [9u8, 10, 11] {
        let _ = gpio.configure_alt(pin, Function::Alt0, Pull::None);
    }
    timer::busy_wait_us(1);
}
