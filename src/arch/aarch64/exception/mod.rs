//! AArch64 exception vectors and trap handling.

mod frame;
mod handlers;

#[allow(unused_imports)] // public API for future trap inspection
pub use frame::TrapFrame;

// The syndrome registers, read by this module's handlers and by `el0`, which
// used to carry byte-identical private copies of both.
pub use handlers::{read_esr_el1, read_far_el1};

// `TRAP_FRAME_SIZE` comes from `frame.rs`, so the assembly reserves exactly
// what the Rust struct needs and the compiler checks it — the two used to
// carry independent constants (`.equ … 0x110` and an `assert!(… == 264)`)
// with nothing tying them together.
//
// Emitted as its own block: `vectors.s` is included with `options(raw)`
// because its `{`/`}` are AArch64 syntax, not format placeholders, and `raw`
// rules out operand substitution in the same call.
core::arch::global_asm!(
    ".equ TRAP_FRAME_SIZE, {trap_frame_size}",
    trap_frame_size = const frame::TRAP_FRAME_SIZE,
);

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
    unsafe extern "C" {
        static exception_vectors: u8;
    }
    // SAFETY: symbol is provided by `vectors.s` and lives in .text.
    core::ptr::addr_of!(exception_vectors) as u64
}
