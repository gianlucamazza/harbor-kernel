//! Hardware drivers.
//!
//! Drivers are board-agnostic. The BSP supplies addresses, clocks, and pinmux.

#[cfg(target_arch = "aarch64")]
pub mod gicv2;
#[cfg(target_arch = "aarch64")]
pub mod pl011;
#[cfg(target_arch = "aarch64")]
pub mod pm;
#[cfg(target_arch = "aarch64")]
pub mod rng200;
#[cfg(target_arch = "aarch64")]
pub mod sdhci;

#[cfg(target_arch = "x86_64")]
pub mod uart16550;

#[cfg(all(target_arch = "aarch64", feature = "debug-display"))]
pub mod delay;
#[cfg(all(target_arch = "aarch64", feature = "debug-display"))]
pub mod ili9486;
#[cfg(all(target_arch = "aarch64", feature = "debug-display"))]
pub mod pin;
#[cfg(all(target_arch = "aarch64", feature = "debug-display"))]
pub mod spi;
