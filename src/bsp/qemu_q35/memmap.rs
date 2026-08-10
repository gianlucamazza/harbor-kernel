//! Compiled constants for QEMU q35 lab guest (ADR-0011 spirit).

/// COM1 data port (16550). Drivers receive this from the BSP, not magic numbers.
pub const COM1_PORT: u16 = 0x3F8;

/// End of the boot.s identity map (512 × 2 MiB = 1 GiB).
///
/// Board truth for later slices (APIC/MMIO placement). Referenced from
/// `lab_x86` so the constant cannot rot unused.
pub const IDENTITY_RAM_END: usize = 0x4000_0000;
