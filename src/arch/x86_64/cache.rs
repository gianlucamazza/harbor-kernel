//! Cache / TLB maintenance (x86 lab).
//!
//! L0 identity maps with WB-like defaults for RAM; no self-modifying code path
//! and no remaps that need `invlpg`. Operations are real instructions (not
//! silent lies) but are unused on this slice’s call graph.

#![allow(dead_code)] // facade surface; not on L0 call graph

use core::arch::asm;

#[inline]
pub fn dcache_clean_range(_start: usize, _len: usize) {
    // x86 coherent D/I for ordinary WB RAM on this lab path; no range op required.
}

#[inline]
pub fn icache_invalidate_all() {
    // SAFETY: serialising instruction; L0 has no concurrent modifiers of text.
    unsafe {
        asm!("mfence", options(nostack, nomem, preserves_flags));
    }
}

#[inline]
pub fn tlb_invalidate_all() {
    // SAFETY: reload CR3 to flush TLB (including global entries only if PGE clear).
    unsafe {
        asm!(
            "mov {tmp}, cr3",
            "mov cr3, {tmp}",
            tmp = out(reg) _,
            options(nostack, preserves_flags),
        );
    }
}
