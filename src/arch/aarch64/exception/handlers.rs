//! Exception handlers — thin: diagnose fatal traps; forward IRQs to `irq`.

use super::frame::TrapFrame;
use crate::irq;

/// Synchronous exception from EL1 — always fatal in M1.
#[no_mangle]
pub extern "C" fn exception_sync_el1(frame: &TrapFrame) -> ! {
    let esr = read_esr_el1();
    let far = read_far_el1();
    panic!(
        "sync exception EL1\n  ESR={esr:#018x}\n  ELR={:#018x}\n  SPSR={:#018x}\n  FAR={far:#018x}",
        frame.elr, frame.spsr
    );
}

/// Unexpected vector — always fatal.
#[no_mangle]
pub extern "C" fn exception_unexpected(frame: &TrapFrame) -> ! {
    let esr = read_esr_el1();
    let far = read_far_el1();
    panic!(
        "unexpected exception\n  ESR={esr:#018x}\n  ELR={:#018x}\n  SPSR={:#018x}\n  FAR={far:#018x}",
        frame.elr, frame.spsr
    );
}

/// IRQ from EL1h → kernel IRQ subsystem (no device knowledge here).
#[no_mangle]
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
