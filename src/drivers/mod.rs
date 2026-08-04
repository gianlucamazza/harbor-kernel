//! Hardware drivers.
//!
//! Drivers are board-agnostic. The BSP supplies addresses, clocks, and pinmux.

pub mod gicv2;
pub mod pl011;
