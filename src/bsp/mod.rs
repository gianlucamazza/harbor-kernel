//! Board support packages.
//!
//! Select the active board with a Cargo `board-*` feature (default:
//! `board-rpi4`). Kernel and driver code depend only on the public BSP surface
//! via [`board`] — never a concrete board path such as `crate::bsp::rpi4`.
//!
//! Adding a board: see [`docs/porting.md`](../../docs/porting.md).

#[cfg(feature = "board-rpi4")]
pub mod rpi4;

#[cfg(feature = "board-qemu-q35")]
pub mod qemu_q35;

/// Active board for this build.
#[cfg(all(feature = "board-rpi4", not(feature = "board-qemu-q35")))]
pub mod board {
    pub use super::rpi4::*;
}

#[cfg(all(feature = "board-qemu-q35", not(feature = "board-rpi4")))]
pub mod board {
    pub use super::qemu_q35::*;
}

#[cfg(all(feature = "board-rpi4", feature = "board-qemu-q35"))]
compile_error!(
    "harbor-kernel: enable exactly one board-* feature (board-rpi4 or board-qemu-q35)"
);

#[cfg(not(any(feature = "board-rpi4", feature = "board-qemu-q35")))]
compile_error!(
    "harbor-kernel: no board selected — enable a board-* feature \
     (default includes board-rpi4; see docs/porting.md)"
);
