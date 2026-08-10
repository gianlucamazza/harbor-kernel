//! Timer role — no APIC/HPET on L0; deadline ops are non-functional no-ops
//! (not armed). A later slice replaces these with a real clocksource.

#![allow(dead_code)] // facade surface; not on L0 call graph

pub fn arm_deadline(_deadline: u64) {}
pub fn status() -> u64 {
    0
}
pub fn rearm() {}
