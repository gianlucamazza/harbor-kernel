//! Board support packages.
//!
//! Select the active board with a Cargo `board-*` feature (default:
//! `board-rpi4`). Kernel and driver code depend only on the public BSP surface
//! via [`board`] — never a concrete board path such as `crate::bsp::rpi4`.
//!
//! Adding a board: see [`docs/porting.md`](../../docs/porting.md).

#[cfg(feature = "board-rpi4")]
pub mod rpi4;

/// Active board for this build.
#[cfg(feature = "board-rpi4")]
pub mod board {
    pub use super::rpi4::*;
}

#[cfg(not(feature = "board-rpi4"))]
compile_error!(
    "harbor-kernel: no board selected — enable a board-* feature \
     (default includes board-rpi4; see docs/porting.md)"
);
