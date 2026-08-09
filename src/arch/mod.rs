//! Architecture facade.
//!
//! Kernel code outside this tree must import only `crate::arch::{…}` — never an
//! ISA path such as `crate::arch::aarch64`. Selection is compile-time
//! (`target_arch`); there is no runtime arch switch and no `dyn Arch`.
//!
//! Contract for ports: [`docs/arch-contract.md`](../../docs/arch-contract.md).
//! How to add an ISA or board: [`docs/porting.md`](../../docs/porting.md).
//!
//! **Supported today:** AArch64 only (product board: Raspberry Pi 4 via BSP).

#[cfg(target_arch = "aarch64")]
mod aarch64;

#[cfg(target_arch = "aarch64")]
pub use aarch64::{bootinfo, cache, cpu, el0, exception, mmio, mmu, probe, smp, switch, timer};

#[cfg(not(target_arch = "aarch64"))]
compile_error!(
    "harbor-kernel: unsupported target_arch — add src/arch/<isa>, wire this facade, \
     and update docs/arch-contract.md (see docs/porting.md)"
);
