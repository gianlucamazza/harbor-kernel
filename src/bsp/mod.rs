//! Board support packages.
//!
//! Select the active board here. Kernel and driver code depend only on the
//! public BSP surface (`console`, memory map constants as needed).

pub mod rpi4;

/// Active board for this build.
pub mod board {
    pub use super::rpi4::*;
}
