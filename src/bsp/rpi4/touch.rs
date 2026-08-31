//! Optional XPT2046/ADS7846 touch input for the Waveshare-class panel.

use kernel_core::touch::{self, Calibration, Orientation};
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
    InvalidCalibration,
}

struct Touch {
    controller: Xpt2046<Device>,
    irq: gpio::Input,
    calibration: Calibration,
    orientation: Orientation,
    pressed: bool,
    last: (u16, u16),
    samples: u32,
    errors: u32,
    last_sample_tick: u64,
    last_irq_low: bool,
    candidate: Option<(u16, u16)>,
    stable_samples: u8,
}

static TOUCH: Mutex<Option<Touch>> = Mutex::new(None);

/// Install the controller with a calibration selected by bootstrap storage.
pub unsafe fn init_with_calibration(calibration: Calibration) -> Result<(), TouchError> {
    if !calibration.is_valid() {
        return Err(TouchError::InvalidCalibration);
    }
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
                calibration,
                orientation: Orientation::Normal,
                pressed: false,
                last: (0, 0),
                samples: 0,
                errors: 0,
                last_sample_tick: 0,
                last_irq_low: false,
                candidate: None,
                stable_samples: 0,
            });
        });
    }
    Ok(())
}

/// Sample at most one event and retain the raw sample for calibration UI.
pub fn poll_with_sample() -> Option<(TouchEvent, touch::RawSample)> {
    crate::bsp::board::spi_display::with_spi0(|| {
        TOUCH.with(|slot| {
            let touch = slot.as_mut()?;
            touch.last_irq_low = !touch.irq.is_high();
            let now = crate::time::ticks();
            // An idle controller keeps IRQ high. Avoiding a SPI transaction in
            // that state leaves the bus available to the panel and makes the
            // sampling rate proportional to actual input activity. Continue
            // sampling while pressed so the high transition can emit `Up`.
            if !touch.last_irq_low && !touch.pressed {
                return None;
            }
            // `on_idle` can run much faster than the panel's useful sample
            // rate. One bounded transaction per timer tick prevents the
            // controller and the UI from seeing duplicate noisy samples.
            if now == touch.last_sample_tick {
                return None;
            }
            touch.last_sample_tick = now;
            let sample = match touch.controller.sample_filtered() {
                Ok(sample) => sample,
                Err(_) => {
                    touch.errors = touch.errors.saturating_add(1);
                    return None;
                }
            };
            if sample.pressure < 32 {
                touch.candidate = None;
                touch.stable_samples = 0;
                if touch.pressed {
                    touch.pressed = false;
                    return Some((
                        TouchEvent {
                            kind: TouchKind::Up,
                            x: touch.last.0,
                            y: touch.last.1,
                        },
                        sample,
                    ));
                }
                return None;
            }
            let kind = if touch.pressed {
                TouchKind::Move
            } else {
                TouchKind::Down
            };
            let event = touch::event_oriented(kind, sample, touch.calibration, touch.orientation);
            if !touch.pressed {
                let stable = touch
                    .candidate
                    .map(|last| close_to(last, (event.x, event.y)))
                    .unwrap_or(false);
                touch.stable_samples = if stable {
                    touch.stable_samples.saturating_add(1)
                } else {
                    1
                };
                touch.candidate = Some((event.x, event.y));
                if touch.stable_samples < 2 {
                    return None;
                }
            }
            touch.pressed = true;
            touch.last = (event.x, event.y);
            touch.samples = touch.samples.saturating_add(1);
            Some((event, sample))
        })
    })
}

/// Apply a validated calibration between samples; no SPI transaction occurs.
pub fn set_calibration(calibration: Calibration) -> bool {
    if !calibration.is_valid() {
        return false;
    }
    TOUCH.with(|slot| {
        let Some(touch) = slot.as_mut() else {
            return false;
        };
        touch.calibration = calibration;
        touch.candidate = None;
        touch.stable_samples = 0;
        true
    })
}

fn close_to(a: (u16, u16), b: (u16, u16)) -> bool {
    a.0.abs_diff(b.0) <= 8 && a.1.abs_diff(b.1) <= 8
}

/// Return counters after the SPI transaction has been released.
pub fn metrics() -> (u32, u32, bool) {
    TOUCH.with(|slot| {
        slot.as_ref()
            .map(|touch| (touch.samples, touch.errors, touch.last_irq_low))
            .unwrap_or((0, 0, false))
    })
}
