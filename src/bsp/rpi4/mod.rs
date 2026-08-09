//! Board support: Raspberry Pi 4 Model B (BCM2711).

pub mod console;
pub mod gpio;
pub mod irq;
pub mod memmap;
pub mod pm;
pub mod rng;
pub mod sdhci;

#[cfg(feature = "debug-display")]
pub mod display;
