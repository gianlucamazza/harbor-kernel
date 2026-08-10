//! SMP role. L0 is single-core; unpark refuses (returns false).

#![allow(dead_code)] // facade surface; not on L0 call graph

pub fn unpark_core1() -> bool {
    false
}

pub fn secondary_seen_count() -> u64 {
    0
}
