//! AArch64 architecture layer.
//!
//! Contains only CPU- and ISA-level facilities. Device drivers and board
//! wiring live in [`crate::drivers`] and [`crate::bsp`] respectively.
//!
//! Boot entry (`boot.s`) and the linker script (`link.ld`) live here so a
//! future ISA port adds a sibling tree rather than root-level artefacts.

pub mod bootinfo;
pub mod cache;
pub mod cpu;
pub mod el0;
pub mod exception;
pub mod mmio;
pub mod mmu;
pub mod probe;
pub mod smp;
pub mod switch;
pub mod timer;

// Early entry: DTB stash, EL2→EL1, early MMU, BSS, stack → `kernel_main`.
// Included from the ISA module so `main` never names `aarch64`.
core::arch::global_asm!(include_str!("boot.s"));
