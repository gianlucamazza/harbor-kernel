//! Board support: Raspberry Pi 4 Model B (BCM2711).

pub mod console;
pub mod display;
pub mod gpio;
pub mod irq;
pub mod memmap;
pub mod net;
pub mod pm;
pub mod rng;
pub mod sdhci;
#[cfg(feature = "debug-display")]
pub mod spi_display;
#[cfg(feature = "display-touch")]
pub mod touch;
