//! Hardware drivers.
//!
//! Drivers are board-agnostic. The BSP supplies addresses, clocks, and pinmux.

pub mod gicv2;
pub mod pl011;
pub mod pm;
pub mod rng200;

#[cfg(feature = "debug-display")]
pub mod delay;
#[cfg(feature = "debug-display")]
pub mod ili9486;
#[cfg(feature = "debug-display")]
pub mod pin;
#[cfg(feature = "debug-display")]
pub mod spi;
