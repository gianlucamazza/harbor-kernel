//! Synchronisation primitives for kernel-global state.
//!
//! # SMP discipline (ADR-0077)
//!
//! Shared mutable kernel data is entered only as:
//!
//! 1. mask local IRQs (`irq_save`);
//! 2. acquire a spinlock;
//! 3. mutate;
//! 4. release the spinlock;
//! 5. restore IRQs.
//!
//! IRQ mask alone is **not** enough when a second core can run the same path.
//! A spinlock alone is **not** enough: an IRQ on the holding core could re-enter
//! the same lock. Device/SGI handlers never take these locks (ADR-0008).
//!
//! [`SyncCell`] remains "Sync by assertion": callers still establish exclusivity
//! (typically via [`IrqSpinLock::with`]).

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::cpu;

/// A cell that is `Sync` by assertion rather than by synchronisation.
///
/// # Safety contract
///
/// Callers must establish exclusivity — for SMP-shared data that means
/// [`IrqSpinLock::with`] (or an equivalent mask+spin section). A bare
/// [`SyncCell`] does not serialise two CPUs.
pub struct SyncCell<T> {
    inner: UnsafeCell<T>,
}

// SAFETY: see the contract above — exclusivity is the caller's obligation.
unsafe impl<T: Send> Sync for SyncCell<T> {}

impl<T> SyncCell<T> {
    /// Wrap `value`.
    pub const fn new(value: T) -> Self {
        Self {
            inner: UnsafeCell::new(value),
        }
    }

    /// Raw pointer to the contents.
    ///
    /// # Safety
    ///
    /// The caller must not create aliasing `&mut` references, and must hold
    /// the exclusivity described in the type contract.
    pub const fn get(&self) -> *mut T {
        self.inner.get()
    }
}

/// Test-and-set spinlock that always runs with local IRQs masked.
///
/// Use for every SMP-shared mutable structure (heap, scheduler, IPC, frame
/// pool, name/storage/taskcap tables, console TX, durable region, …). Do
/// **not** call from an IRQ handler that might nest against a holder on the
/// same core.
///
/// # Lock order (deadlock avoidance)
///
/// When two of these locks can nest on a path, the order is fixed:
/// **IPC → SCHED** is allowed only as separate critical sections (IPC dropped
/// before `wake_task` / `block_current`). Never hold **SCHED** while taking
/// **IPC** (spawn registers holds after `with_sched` returns). Other global
/// tables (naming, storage, taskcap, frames, asid, durable, TX) do not nest
/// under each other or under SCHED on current product paths.
pub struct IrqSpinLock {
    locked: AtomicBool,
}

impl IrqSpinLock {
    /// Unlocked lock.
    pub const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
        }
    }

    /// Run `f` under IRQ mask + exclusive spin.
    #[inline]
    pub fn with<R>(&self, f: impl FnOnce() -> R) -> R {
        let daif = cpu::irq_save();
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        let r = f();
        self.locked.store(false, Ordering::Release);
        // SAFETY: closes the section opened above.
        unsafe { cpu::irq_restore(daif) };
        r
    }

    /// Acquire without restoring IRQs — caller already masked and will restore.
    ///
    /// Used by the scheduler switch path, which must keep IRQs masked across
    /// `context_switch` while releasing the lock before the stack swap.
    #[inline]
    pub fn lock_already_masked(&self) {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    /// Release after [`Self::lock_already_masked`].
    #[inline]
    pub fn unlock_already_masked(&self) {
        self.locked.store(false, Ordering::Release);
    }
}
