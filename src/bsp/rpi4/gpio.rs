//! BCM2711 GPIO controller — pinmux, pull, and output drive.
//!
//! Board-specific register layout.
//!
//! Reference: BCM2711 ARM Peripherals (GPFSEL / GPPUPPDN). The output-drive
//! half — GPSET/GPCLR, `Function::Output`, the SPI0 data pinmux — went with the
//! panel it existed for (ADR-0094). UART0 is what remains.

use crate::arch::mmio::Mmio;
use crate::arch::timer;
use crate::bsp::rpi4::memmap::GPIO_BASE;

/// Highest GPIO index this module will touch (BCM2711 exposes 0–57).
pub const PIN_MAX: u8 = 57;

const GPFSEL0: usize = 0x00;
const GPPUPPDN0: usize = 0xE4;

/// Pin function select (3-bit field in GPFSELn).
///
/// Only the encodings in use are listed; add others when a peripheral needs them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Function {
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
            // SAFETY: `GPIO_BASE` is this board's GPIO block, inside the
            // peripheral window `mm::layout` maps Device-nGnRnE. The
            // "one owner" obligation is this constructor's own `# Safety`.
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

    /// Configure `pin` for an alternate function (SPI, UART, …).
    pub fn configure_alt(&self, pin: u8, function: Function, pull: Pull) -> Result<(), GpioError> {
        self.set_function(pin, function)?;
        self.set_pull(pin, pull)?;
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
    // SAFETY: forwarded from this function's `# Safety` — the caller owns the
    // GPIO block for the duration, and the handle does not outlive this call.
    let gpio = unsafe { Gpio::new() };
    // UART0 is ALT0 on 14/15; pins are in range by construction.
    let _ = gpio.configure_alt(14, Function::Alt0, Pull::None);
    let _ = gpio.configure_alt(15, Function::Alt0, Pull::None);
    // Wall-time settle (~1 µs) rather than a CPU-cycle guess.
    timer::busy_wait_us(1);
}
