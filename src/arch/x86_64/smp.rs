//! SMP role. L0 is single-core; unpark refuses (returns false).

#![allow(dead_code)] // facade surface; not on L0 call graph

pub fn unpark_core1() -> bool {
    false
}

pub fn secondary_seen_count() -> u64 {
    0
}

pub fn release_secondary_irq_bringup() {}

pub fn secondary_may_irq() -> bool {
    false
}

pub fn mark_secondary_irq_ready() {}

pub fn wait_secondary_irq_ready(_budget: u64) -> bool {
    false
}

pub fn note_core1_ipi() {}

pub fn wait_core1_ipi(_budget: u64) -> bool {
    false
}
