//! H3 L0 lab bring-up for x86_64 QEMU (ADR-0071).
//!
//! Thin path (progressive-isa P.2–P.3): COM1 banner + CPUID line + alive + halt.

use core::fmt::Write;

use kernel_core::cpuid::{x86_display_family, x86_display_model};

use crate::arch::cpu;
use crate::bsp::board;

/// Board truth: boot.s identity map covers `[0, IDENTITY_RAM_END)`.
const _: usize = board::memmap::IDENTITY_RAM_END;

/// Lab entry after PVH → long mode.
pub fn run() -> ! {
    // SAFETY: sole owner of COM1 on this path.
    let mut uart = unsafe { board::console::bind() };

    let _ = writeln!(uart, "Harbor: hello (x86 lab)");
    let _ = writeln!(uart, "build: x86-lab L0 (ADR-0071)");

    let vendor = cpu::vendor_id();
    let vendor_str = core::str::from_utf8(&vendor).unwrap_or("????????");
    let leaf1 = cpu::cpuid_leaf1_eax();
    let family = x86_display_family(leaf1);
    let model = x86_display_model(leaf1);
    let _ = writeln!(
        uart,
        "cpu: {vendor_str} family={family} model={model} (eax1={leaf1:#010x})"
    );

    let _ = writeln!(uart, "x86-lab: alive");

    cpu::halt()
}
