//! Synchronisation primitives for kernel-global state.
//!
//! [`SyncCell`] is still "Sync by assertion": callers must establish exclusivity.
//! Since ADR-0076, schedule state is guarded by a real spinlock in `sched` (IRQs
//! masked on the local core while the lock is held). Other globals still rely on
//! the single-writer-or-IRQ-masked discipline documented on each site.

use core::cell::UnsafeCell;

/// A cell that is `Sync` by assertion rather than by synchronisation.
///
/// # Safety contract
///
/// Callers must establish exclusivity themselves — typically local IRQ mask
/// plus, for `SCHED`, the sched spinlock (ADR-0075/0076). A bare `SyncCell`
/// does not serialise two CPUs.
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
