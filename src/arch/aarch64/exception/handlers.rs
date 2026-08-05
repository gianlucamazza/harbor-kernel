//! Exception handlers — thin: diagnose fatal traps; forward IRQs to `irq`.
//!
//! Sync EL1t may return when [`crate::arch::probe`] consumes a deliberate MMIO
//! presence fault; the vector path then `eret`s (see `vectors.s`).

// Audit debt (2026-08-06): 2 unsafe blocks here predate
// `clippy::undocumented_unsafe_blocks` and do not yet say what makes them sound.
// This comes off when the audit reaches this module and the SAFETY comments can
// state something checkable rather than restate the code. See Cargo.toml.
#![allow(clippy::undocumented_unsafe_blocks)]

use super::frame::TrapFrame;
use crate::arch::probe;
use crate::irq;

/// Synchronous exception from EL1.
///
/// Most traps are fatal. An active [`probe`] window that matches this data
/// abort advances `ELR` by one A64 instruction and returns so the access can
/// report `Err` instead of hanging the board.
#[unsafe(no_mangle)]
pub extern "C" fn exception_sync_el1(frame: &mut TrapFrame) {
    let esr = read_esr_el1();
    let far = read_far_el1();
    if probe::take_data_abort(far, esr) {
        // A64 fixed-length encoding: skip the faulting LDR/STR.
        frame.elr = frame.elr.wrapping_add(4);
        return;
    }
    panic!(
        "sync exception EL1\n  ESR={esr:#018x}\n  ELR={:#018x}\n  SPSR={:#018x}\n  FAR={far:#018x}",
        frame.elr, frame.spsr
    );
}

/// Unexpected vector — always fatal.
#[unsafe(no_mangle)]
pub extern "C" fn exception_unexpected(frame: &TrapFrame) -> ! {
    let esr = read_esr_el1();
    let far = read_far_el1();
    panic!(
        "unexpected exception\n  ESR={esr:#018x}\n  ELR={:#018x}\n  SPSR={:#018x}\n  FAR={far:#018x}",
        frame.elr, frame.spsr
    );
}

/// IRQ from EL1h → kernel IRQ subsystem (no device knowledge here).
#[unsafe(no_mangle)]
pub extern "C" fn exception_irq_el1() {
    irq::handle_cpu_irq();
}

#[inline(always)]
fn read_esr_el1() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {}, esr_el1",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}

#[inline(always)]
fn read_far_el1() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {}, far_el1",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}
