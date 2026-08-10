//! x86_64 architecture layer (lab / H3 L0 — ADR-0071).
//!
//! Full facade re-export for the arch contract. L0 call graph uses `cpu` and
//! `mmio` (plus boot.s). Other modules are progressive surfaces: uncalled APIs
//! refuse or panic if entered — they do not silent-succeed (progressive-isa P.4–P.5).

pub mod bootinfo;
pub mod cache;
pub mod cpu;
pub mod el0;
pub mod exception;
pub mod mmio;
pub mod mmu;
pub mod probe;
pub mod smp;
pub mod switch;
pub mod timer;

core::arch::global_asm!(include_str!("boot.s"), options(att_syntax));
