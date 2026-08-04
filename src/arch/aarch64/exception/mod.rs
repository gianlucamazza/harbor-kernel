//! AArch64 exception vectors and trap handling.

mod frame;
mod handlers;

#[allow(unused_imports)] // public API for future trap inspection
pub use frame::TrapFrame;

core::arch::global_asm!(include_str!("vectors.s"), options(raw));

/// Install the vector table into `VBAR_EL1`.
///
/// Must run before unmasking IRQs. Safe to call once at boot on core 0.
pub fn init() {
    let vectors = exception_vectors_addr();
    // SAFETY: `exception_vectors` is 2 KiB-aligned in the linker script.
    unsafe {
        core::arch::asm!(
            "msr vbar_el1, {vbar}",
            "isb",
            vbar = in(reg) vectors,
            options(nostack, preserves_flags),
        );
    }
}

#[inline(always)]
fn exception_vectors_addr() -> u64 {
    extern "C" {
        static exception_vectors: u8;
    }
    // SAFETY: symbol is provided by `vectors.s` and lives in .text.
    core::ptr::addr_of!(exception_vectors) as u64
}
