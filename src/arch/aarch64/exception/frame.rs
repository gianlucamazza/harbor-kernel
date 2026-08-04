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

// 31*8 + 8 + 8 = 264; keep 16-byte stack alignment with explicit padding in asm.
const _: () = assert!(core::mem::size_of::<TrapFrame>() == 264);
