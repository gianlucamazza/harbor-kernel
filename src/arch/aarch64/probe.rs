//! Recoverable MMIO probe for device presence.
//!
//! A write to a Device mapping that has no backend (emulator hole, powered-down
//! block) is a synchronous external abort — not a soft error the driver can
//! see through `read_volatile`. This module opens a **one-instruction window**
//! where a data abort at a known address is consumed: the sync handler advances
//! `ELR` past the faulting A64 instruction and the caller gets `Err`.
//!
//! Used after the MMU is on (exclusives and Device attrs are well-defined).
//! Not a substitute for real fault handling: anything outside an active probe
//! still panics.

use core::ptr::write_volatile;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::cpu;

/// Probe in progress on this core (single-core boot: one global is enough).
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Set by the sync handler when it consumed a probe fault.
static FAULTED: AtomicBool = AtomicBool::new(false);
/// Physical/virt address expected to fault (the MMIO location being probed).
static ADDR: AtomicUsize = AtomicUsize::new(0);

/// ESR.EC for Data Abort from the current EL.
const ESR_EC_DATA_ABORT_CURRENT: u64 = 0x25;
/// DFSC: Synchronous External Abort (not on a translation table walk).
const DFSC_SYNC_EXTERNAL: u64 = 0x10;

/// Called from the EL1 sync handler before treating a trap as fatal.
///
/// Returns `true` if this abort was an active probe and has been recorded:
/// the handler must add 4 to `ELR` (A64 instruction size) and return so the
/// vector path can `eret`.
#[inline]
pub fn take_data_abort(far: u64, esr: u64) -> bool {
    if !ACTIVE.load(Ordering::Relaxed) {
        return false;
    }

    let ec = (esr >> 26) & 0x3f;
    if ec != ESR_EC_DATA_ABORT_CURRENT {
        return false;
    }

    // Only external aborts: translation/permission faults at the wrong place
    // remain fatal so real map bugs still panic.
    let dfsc = esr & 0x3f;
    if dfsc != DFSC_SYNC_EXTERNAL {
        return false;
    }

    let expected = ADDR.load(Ordering::Relaxed) as u64;
    if far != expected {
        return false;
    }

    FAULTED.store(true, Ordering::Relaxed);
    true
}

/// Write `value` to a 32-bit MMIO location; `Err` if the access aborts.
///
/// # Safety
///
/// `addr` must be a Device-mapped MMIO address when the device exists. On
/// success the write has completed; on `Err` the location did not accept it.
pub unsafe fn try_write32(addr: usize, value: u32) -> Result<(), ()> {
    cpu::without_irqs(|| {
        FAULTED.store(false, Ordering::Relaxed);
        ADDR.store(addr, Ordering::Relaxed);
        // Release: handler on this core must see ACTIVE before the access.
        ACTIVE.store(true, Ordering::Release);
        // SAFETY: ordered probe window; fault path clears via take_data_abort.
        unsafe {
            core::arch::asm!("dsb sy", "isb", options(nostack, preserves_flags));
            write_volatile(addr as *mut u32, value);
            core::arch::asm!("dsb sy", "isb", options(nostack, preserves_flags));
        }
        ACTIVE.store(false, Ordering::Release);

        if FAULTED.load(Ordering::Acquire) {
            Err(())
        } else {
            Ok(())
        }
    })
}
