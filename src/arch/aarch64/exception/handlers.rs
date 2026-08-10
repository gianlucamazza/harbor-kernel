//! Exception handlers — thin: diagnose fatal traps; forward IRQs to `irq`.
//!
//! Sync EL1t may return when [`crate::arch::probe`] consumes a deliberate MMIO
//! presence fault; the vector path then `eret`s (see `vectors.s`).

use core::sync::atomic::{AtomicU64, Ordering};

use super::frame::TrapFrame;
use crate::arch::probe;
use crate::irq;

/// The syndrome of the trap that is about to panic, published for the panic
/// handler.
///
/// The handler formats the syndrome it knows how to read; naming the faulting
/// *address* needs the region table, which is policy's and which `arch` may
/// not import (rule 3). So the two halves meet here: `arch` publishes, the
/// panic path decorates. `VALID` is cleared by nobody — a panic from any other
/// path would find a stale record, so it also stores the `ELR` it belongs to
/// and the panic handler only trusts a record it has not seen before.
static FAULT_ESR: AtomicU64 = AtomicU64::new(0);
static FAULT_FAR: AtomicU64 = AtomicU64::new(0);
static FAULT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Syndrome, faulting address and a sequence number that increments per
/// recorded trap. `None` before the first fatal trap.
pub fn last_fault() -> Option<(u64, u64, u64)> {
    match FAULT_SEQ.load(Ordering::Acquire) {
        0 => None,
        seq => Some((
            FAULT_ESR.load(Ordering::Relaxed),
            FAULT_FAR.load(Ordering::Relaxed),
            seq,
        )),
    }
}

/// `ESR_EL1` rendered as the sentence `kernel_core::fault` decodes it, so the
/// first line of a panic says what happened and the hex below it stays for
/// whoever needs the raw bits.
pub struct Syndrome(pub u64);

impl core::fmt::Display for Syndrome {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let fault = kernel_core::fault::describe(self.0);
        f.write_str(fault.class)?;
        if let Some(detail) = fault.detail {
            write!(f, ", {}", detail.kind)?;
            if let Some(level) = detail.level {
                write!(f, " level {level}")?;
            }
            match detail.write {
                Some(true) => f.write_str(" on write")?,
                Some(false) => f.write_str(" on read")?,
                None => {}
            }
        }
        Ok(())
    }
}

fn record_fault(esr: u64, far: u64) {
    FAULT_ESR.store(esr, Ordering::Relaxed);
    FAULT_FAR.store(far, Ordering::Relaxed);
    FAULT_SEQ.fetch_add(1, Ordering::Release);
}

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
    record_fault(esr, far);
    panic!(
        "sync exception EL1: {}\n  ESR={esr:#018x}\n  ELR={:#018x}\n  SPSR={:#018x}\n  FAR={far:#018x}",
        Syndrome(esr),
        frame.elr,
        frame.spsr
    );
}

/// Unexpected vector — always fatal.
#[unsafe(no_mangle)]
pub extern "C" fn exception_unexpected(frame: &TrapFrame) -> ! {
    let esr = read_esr_el1();
    let far = read_far_el1();
    record_fault(esr, far);
    panic!(
        "unexpected exception: {}\n  ESR={esr:#018x}\n  ELR={:#018x}\n  SPSR={:#018x}\n  FAR={far:#018x}",
        Syndrome(esr),
        frame.elr,
        frame.spsr
    );
}

/// IRQ from EL1h → kernel IRQ subsystem (no device knowledge here).
#[unsafe(no_mangle)]
pub extern "C" fn exception_irq_el1() {
    irq::handle_cpu_irq();
}

/// `ESR_EL1` — the syndrome of the exception currently being handled.
///
/// Shared with [`crate::arch::el0`], which needs the same two registers for
/// lower-EL events and had its own byte-identical copies.
#[inline(always)]
pub fn read_esr_el1() -> u64 {
    let value: u64;
    // SAFETY: `ESR_EL1` is readable at EL1 and has no side effects. It is only
    // meaningful inside a handler, before another exception overwrites it —
    // which is what every caller here is.
    unsafe {
        core::arch::asm!(
            "mrs {}, esr_el1",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}

/// `FAR_EL1` — the faulting address for aborts. Meaningless for other classes.
#[inline(always)]
pub fn read_far_el1() -> u64 {
    let value: u64;
    // SAFETY: as [`read_esr_el1`]; a system register read with no side effects.
    unsafe {
        core::arch::asm!(
            "mrs {}, far_el1",
            out(reg) value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}
