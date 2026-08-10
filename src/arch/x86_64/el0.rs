//! User-session role (`el0` name kept per arch-contract P.1).
//!
//! L0 has no ring3; entry points panic if reached (progressive-isa P.4).

#![allow(dead_code)] // facade surface; not on L0 call graph

pub struct El0Session;

pub enum El0Outcome {
    Yield,
}

impl El0Session {
    pub fn publish(_p: *mut Self) {}
    pub fn published() -> *mut Self {
        core::ptr::null_mut()
    }
}

/// # Safety
/// Not implemented on L0.
pub unsafe fn enter(_s: &mut El0Session) -> El0Outcome {
    panic!("x86 L0: el0::enter not implemented")
}

/// # Safety
/// Not implemented on L0.
pub unsafe fn resume(_s: &mut El0Session) -> El0Outcome {
    panic!("x86 L0: el0::resume not implemented")
}

pub fn end_session(_s: &mut El0Session) {}

/// # Safety
/// Not implemented on L0.
pub unsafe fn run(_s: &mut El0Session) -> El0Outcome {
    panic!("x86 L0: el0::run not implemented")
}
