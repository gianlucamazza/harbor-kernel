//! Exception trap frame shared between the vector stubs and Rust handlers.

/// Saved EL1 state for unexpected exceptions and (future) richer IRQ paths.
///
/// Layout must match `vectors.s` (`KERNEL_ENTRY` / `KERNEL_EXIT`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    /// General-purpose registers `x0` … `x30`.
    pub gpr: [u64; 31],
    /// `ELR_EL1` at exception entry.
    pub elr: u64,
    /// `SPSR_EL1` at exception entry.
    pub spsr: u64,
}

/// Bytes `vectors.s` reserves on the stack for a trap frame.
///
/// The stub subtracts this before storing registers, so it must cover the
/// struct and keep `sp` 16-byte aligned. It is defined here and substituted
/// into the assembly by `global_asm!`, so there is one definition: previously
/// the assembly had its own `.equ TRAP_FRAME_SIZE, 0x110` and this file
/// asserted `264`, with nothing tying the two together. Changing the struct
/// corrupted the stack while the assertion kept passing.
pub const TRAP_FRAME_SIZE: usize = size_of::<TrapFrame>().next_multiple_of(16);

// The stubs address every field by explicit offset, so the layout below is
// load-bearing: `gpr` first, then `elr`, then `spsr`.
const _: () = assert!(core::mem::offset_of!(TrapFrame, gpr) == 0);
const _: () = assert!(core::mem::offset_of!(TrapFrame, elr) == 31 * 8);
const _: () = assert!(core::mem::offset_of!(TrapFrame, spsr) == 32 * 8);
const _: () = assert!(TRAP_FRAME_SIZE % 16 == 0);
