//! Synchronisation primitives for kernel-global state.
//!
//! Through M2 the kernel is single-core, so the only real concurrency is
//! between the main loop and the IRQ path. [`SyncCell`] exists to say that in
//! the type system: a `static mut` states the same thing in a comment, and a
//! comment cannot be checked — nor migrated to edition 2024, where references
//! to `static mut` are an error.

use core::cell::UnsafeCell;

/// A cell that is `Sync` by assertion rather than by synchronisation.
///
/// # Safety contract
///
/// Callers must establish exclusivity themselves. Today that means: a single
/// active core, and either IRQs masked or no IRQ-path accessor for the value
/// held inside. When a second core appears this type must be replaced by a
/// real lock, and the compiler will point at every use.
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
