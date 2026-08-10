//! Soft device probe. L0 has no soft-fail MMIO path; always “absent”.

#![allow(dead_code)] // facade surface; not on L0 call graph

pub fn try_probe<R>(_f: impl FnOnce() -> R) -> Option<R> {
    None
}
