//! Architecture facade.
//!
//! Kernel code outside this tree must import only `crate::arch::{…}` — never an
//! ISA path such as `crate::arch::aarch64`. Selection is compile-time
//! (`target_arch`); there is no runtime arch switch and no `dyn Arch`.
//!
//! Contract for ports: [`docs/arch-contract.md`](../../docs/arch-contract.md).
//! How to add an ISA or board: [`docs/porting.md`](../../docs/porting.md).
//!
//! **Product today:** AArch64 + Raspberry Pi 4 BSP.
//! **Lab (H3 L0):** x86_64 + QEMU q35 BSP ([ADR-0071](../../docs/adr/0071-h3-l0-x86-qemu-first-slice.md)).

#[cfg(target_arch = "aarch64")]
mod aarch64;

#[cfg(target_arch = "aarch64")]
pub use aarch64::{bootinfo, cache, cpu, el0, exception, mmio, mmu, probe, smp, switch, timer};

#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "x86_64")]
#[allow(unused_imports)] // Facade contract surface for future slices.
pub use x86_64::{bootinfo, cache, cpu, el0, exception, mmio, mmu, probe, smp, switch, timer};

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
compile_error!(
    "harbor-kernel: unsupported target_arch — add src/arch/<isa>, wire this facade, \
     and update docs/arch-contract.md (see docs/porting.md)"
);
