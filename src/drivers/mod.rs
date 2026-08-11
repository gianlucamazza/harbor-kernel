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
