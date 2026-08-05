//! AArch64 architecture layer.
//!
//! Contains only CPU- and ISA-level facilities. Device drivers and board
//! wiring live in [`crate::drivers`] and [`crate::bsp`] respectively.

pub mod bootinfo;
pub mod cache;
pub mod cpu;
pub mod el0;
pub mod exception;
pub mod mmio;
pub mod mmu;
pub mod probe;
pub mod switch;
pub mod timer;
