//! Exception role — IDT not installed on L0 (progressive-isa L0 ladder).

#![allow(dead_code)] // facade surface; not on L0 call graph

#[repr(C)]
pub struct TrapFrame {
    pub rax: u64,
}

/// No-op until an IDT slice lands. Does not claim vectors are live.
pub fn init() {}
