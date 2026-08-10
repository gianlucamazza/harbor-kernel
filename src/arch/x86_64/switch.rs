//! Cooperative context-switch role. No scheduler on L0.

#![allow(dead_code)] // facade surface; not on L0 call graph

#[repr(C)]
pub struct Context {
    pub rsp: u64,
}

/// # Safety
/// Not implemented on L0.
pub unsafe fn context_switch(_prev: *mut Context, _next: *mut Context) {
    panic!("x86 L0: context_switch not implemented")
}
