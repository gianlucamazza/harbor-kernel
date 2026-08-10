//! Firmware / loader handoff (x86 lab).
//!
//! L0 uses PVH entry only; no Multiboot info pointer and no Device Tree.
//! Optional PVH start_info is not consumed on this slice (ADR-0071 non-goals).

#![allow(dead_code)] // facade surface; not on L0 call graph

/// Survey loader-provided tables (none on L0).
pub fn survey() {}

/// No Device Tree on the lab path (ADR-0011 spirit; progressive-isa 0.3).
pub fn device_tree() -> Option<u64> {
    None
}

pub fn dtb_address() -> u64 {
    0
}

pub fn device_tree_pages() -> Option<(u64, u64)> {
    None
}

/// No blob to consume on the lab path (ADR-0072: the x86 slice builds its
/// description from CPUID/PVH instead).
///
/// # Safety
///
/// Trivially safe (always `None`); unsafe to match the facade signature.
pub unsafe fn device_tree_slice() -> Option<&'static [u8]> {
    None
}
