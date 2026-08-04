//! Architecture abstraction.
//!
//! Today only AArch64 is supported (Raspberry Pi 4). Future ports add a
//! sibling module and select it from the BSP.

pub mod aarch64;

pub use aarch64::bootinfo;
pub use aarch64::cpu;
pub use aarch64::exception;
pub use aarch64::mmio;
pub use aarch64::mmu;
pub use aarch64::switch;
pub use aarch64::timer;
