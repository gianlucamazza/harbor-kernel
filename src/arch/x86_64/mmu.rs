//! MMU surface for x86 lab (ADR-0071).
//!
//! **L0 ownership:** early identity paging is built and activated in `boot.s`
//! (CR0.PG, 1 GiB of 2 MiB pages). This module does **not** rebuild that map.
//! Dynamic map/unmap/user-root switch are not on L0 — they refuse, they do not
//! pretend success (progressive-isa P.5 / P.6).

#![allow(dead_code)] // facade surface; L0 call graph does not use these yet

use kernel_core::layout::Region;

/// Errors for dynamic map operations. L0 has no Rust-owned page-table arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MmuError {
    Unaligned { va: u64, pa: u64, len: u64 },
    BadDescriptor { va: u64, pa: u64 },
    OutOfRange(u64),
    OutOfTables,
    BlockAlreadyMapped(u64),
    /// Boot.s owns the only map; Rust has not taken over.
    NotActivated,
    AlreadyMapped(u64),
    NotMapped(u64),
}

/// Boot path already installed an identity map; nothing for Rust to re-enable.
///
/// # Safety
/// Callers must not assume this builds tables — only that long-mode paging is
/// already live after `_start`.
pub unsafe fn enable_identity(_root: u64) {}

/// L0: identity map is already active. Does not install `regions` (no arena).
///
/// # Safety
/// Same as boot-time identity: valid only while boot.s tables remain in CR3.
pub unsafe fn activate(_regions: &[Region]) -> Result<(), (MmuError, &'static str)> {
    // Honest: we do not apply `regions`. Callers that need a Rust-owned map
    // must wait for a later slice. Returning Ok documents "paging is on", not
    // "regions were installed" — activate on AArch64 both enables and applies;
    // on L0 x86 enable already happened in asm. See progressive-isa P.5.
    Ok(())
}

/// # Safety
/// No dynamic mapper on L0.
pub unsafe fn map(_region: &Region) -> Result<(), MmuError> {
    Err(MmuError::OutOfTables)
}

/// # Safety
/// No dynamic mapper on L0.
pub unsafe fn unmap(_va: u64, _len: u64) -> Result<(), MmuError> {
    Err(MmuError::NotMapped(0))
}

/// Physical root of the **boot** page tables is not exported yet (lives in asm).
pub fn kernel_root_phys() -> Option<usize> {
    None
}

/// No user address-space switch on L0.
///
/// # Safety
/// Must not be used to claim TTBR-equivalent semantics.
pub unsafe fn switch_ttbr0(_ttbr: u64) {
    panic!("x86 L0: switch_ttbr0 not implemented (no user AS)")
}

/// Paging is on after boot.s (CR0.PG). Not a claim that Rust owns the tables.
#[inline]
pub fn is_enabled() -> bool {
    true
}

pub fn tables_remaining() -> usize {
    0
}

pub fn tables_free() -> usize {
    0
}

pub fn splits() -> u32 {
    0
}
