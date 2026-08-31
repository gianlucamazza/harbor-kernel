//! Optional XPT2046/ADS7846 touch input for the Waveshare-class panel.

use kernel_core::touch::{self, Calibration};
use kernel_core::ui::{TouchEvent, TouchKind};

use crate::arch::mmio::Mmio;
use crate::bsp::rpi4::{gpio, memmap};
use crate::drivers::delay::ArchTimerDelay;
use crate::drivers::spi::{BcmSpi, ExclusiveDevice};
use crate::drivers::touch::Xpt2046;
use crate::sync::Mutex;

/// Touch controller chip-select (BCM GPIO 7).
pub const TOUCH_CS_PIN: u8 = 7;
/// Active-low pen interrupt (BCM GPIO 17).
pub const TOUCH_IRQ_PIN: u8 = 17;

type Device = ExclusiveDevice<BcmSpi, gpio::Output, ArchTimerDelay>;
/// Why the optional touch controller could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchError {
    Clock(kernel_core::spi::ClockDivError),
    Gpio(gpio::GpioError),
    Pin(core::convert::Infallible),
}

struct Touch {
    controller: Xpt2046<Device>,
    irq: gpio::Input,
    calibration: Calibration,
    pressed: bool,
}

static TOUCH: Mutex<Option<Touch>> = Mutex::new(None);

/// Install the optional touch controller. Failure is reported by the caller;
/// the display remains usable without it.
pub unsafe fn init() -> Result<(), TouchError> {
    let cdiv = kernel_core::spi::clock_divisor(memmap::SPI0_CORE_CLOCK_HZ, memmap::SPI0_TARGET_HZ)
        .map_err(TouchError::Clock)?;

    // SAFETY: early bootstrap owns GPIO and SPI0 before the scheduler admits
    // any agent. Touch and panel transactions are serialized by their callers.
    unsafe {
        let gpio = gpio::Gpio::new();
        gpio::configure_spi0_data_pins(&gpio);
        let cs = gpio
            .claim_output(TOUCH_CS_PIN, gpio::Pull::None)
            .map_err(TouchError::Gpio)?;
        let irq = gpio
            .claim_input(TOUCH_IRQ_PIN, gpio::Pull::Up)
            .map_err(TouchError::Gpio)?;
        let bus = BcmSpi::init(Mmio::new(memmap::SPI0_BASE), cdiv);
        let device = ExclusiveDevice::new(bus, cs, ArchTimerDelay, 0).map_err(TouchError::Pin)?;
        TOUCH.with(|slot| {
            *slot = Some(Touch {
                controller: Xpt2046::new(device),
                irq,
                calibration: Calibration::DEFAULT,
                pressed: false,
            });
        });
    }
    Ok(())
}

/// Sample at most one event. The IRQ line makes the common idle path free.
pub fn poll() -> Option<TouchEvent> {
    TOUCH.with(|slot| {
        let touch = slot.as_mut()?;
        let down = !touch.irq.is_high();
        if !down {
            if touch.pressed {
                touch.pressed = false;
                return Some(TouchEvent {
                    kind: TouchKind::Up,
                    x: 0,
                    y: 0,
                });
            }
            return None;
        }
        let sample = touch.controller.sample().ok()?;
        if sample.pressure == 0 {
            return None;
        }
        let kind = if touch.pressed {
            TouchKind::Move
        } else {
            TouchKind::Down
        };
        touch.pressed = true;
        Some(touch::event(kind, sample, touch.calibration))
    })
}
