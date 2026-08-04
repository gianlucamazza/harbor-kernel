//! AArch64 architecture layer.
//!
//! Contains only CPU- and ISA-level facilities. Device drivers and board
//! wiring live in [`crate::drivers`] and [`crate::bsp`] respectively.

pub mod cache;
pub mod cpu;
pub mod exception;
pub mod mmio;
pub mod mmu;
pub mod timer;
