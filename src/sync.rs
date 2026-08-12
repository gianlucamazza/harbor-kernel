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
//! [`Mutex`] is that shape with the datum **inside** it (ADR-0091): the lock
//! names what it protects, and the `unsafe` deref lives here once instead of at
//! every call site. [`SyncCell`] is what remains for the three statics a
//! `Mutex` cannot hold — see its doc.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::cpu;

/// A cell that is `Sync` by assertion rather than by synchronisation.
///
/// # This is the exception, not the default
///
/// Shared kernel state is a [`Mutex`]. `SyncCell` is for the statics a `Mutex`
/// cannot hold, and ADR-0091 §4 enumerates them — **all three**:
///
/// - `irq::STATE` — read from the IRQ dispatch path after `seal()`; a handler
///   must never take a lock (ADR-0008), so the exclusion cannot be a lock.
/// - `bootstrap::loader::NAME_POOL` and `STORE_ENTRIES` — `try_store_manifest`
///   mints `&'static str` / `&'static [AgentEntry]` out of them, and a guard
///   cannot lend its interior for `'static`. Boot window only.
///
/// A **fourth user requires an ADR**. Reaching for this type because `Mutex`
/// was inconvenient is the thing ADR-0091 exists to refuse.
///
/// # Safety contract
///
/// Callers must establish exclusivity themselves. A bare `SyncCell` does not
/// serialise two CPUs.
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

/// A value that is only reachable under IRQ mask + exclusive spin (ADR-0091).
///
/// **Not a sleeping mutex.** There is no blocking primitive in this kernel:
/// this spins with local IRQs masked, and holding one across a task switch is
/// exactly what ADR-0022 forbids. Do **not** take one from an IRQ handler that
/// could nest against a holder on the same core (ADR-0008).
///
/// Use for every SMP-shared mutable structure (heap, scheduler, IPC, frame
/// pool, name/storage/taskcap tables, console TX, durable region, …). The lock
/// owns the datum, so "which lock guards this?" is answered by the type rather
/// than by two statics agreeing on a name.
///
/// # Lock order (deadlock avoidance)
///
/// When two of these can nest on a path, the order is fixed:
///
/// - **IPC → SCHED** only as *separate* critical sections (IPC dropped before
///   `wake_task` / `block_current`). Never hold **SCHED** while taking **IPC**
///   (spawn registers holds after `with_sched` returns).
/// - **WAIT → SCHED** likewise separate: the drain pops under WAIT, then wakes
///   under SCHED.
/// - **LINE → TX** genuinely nests, on the live RX-handover path:
///   `console::suspend_rx` / `resume_rx` apply their plan steps inside the line
///   lock, and each step reaches the TX handle. TX is a leaf and the direction
///   is one-way. (ADR-0077 omitted this; ADR-0091 §5 records it.)
///
/// - **SIDE → SCHED** on the loader's spawn path: `loader::load_all` holds its
///   side tables across `sched::spawn_with_slots_on` so a task admitted to the
///   *other* CPU's queue cannot reach `agent_body` before its manifest entry is
///   recorded. Without the nesting the window is real and was seen (2026-08-11,
///   `loader: a task reached the agent body with no manifest entry`). One way:
///   nothing holding SCHED ever takes SIDE.
///
/// Other global tables (naming, storage, taskcap, frames, asid, durable) do not
/// nest under each other or under SCHED on current paths.
pub struct Mutex<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: `value` is reachable only through `with`/`lock_masked`, which
// serialise two CPUs (spin) and the holding core against itself (IRQ mask).
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// An unlocked mutex holding `value`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Run `f` on the guarded value under IRQ mask + exclusive spin.
    ///
    /// The ordinary entry point. Built on [`cpu::without_irqs`] rather than a
    /// hand-rolled `irq_save`/`irq_restore` pair: the mask sequence keeps
    /// exactly one definition (`arch::cpu`), and the region stays visible to
    /// the `irq-scope` gate's scope walker, which only opens `without_irqs(`.
    ///
    /// There is deliberately **no** `lock()` that masks on acquire and restores
    /// on `Drop`: a region with no lexical opener is a region that walker
    /// cannot see into (ADR-0091, rejected alternatives).
    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        cpu::without_irqs(|| {
            self.raw_lock();
            // SAFETY: IRQs are masked (no same-core re-entry) and the spin bit
            // is ours (no other core), so this is the only live reference. It
            // ends with `f`.
            let r = f(unsafe { &mut *self.value.get() });
            self.raw_unlock();
            r
        })
    }

    /// Acquire without touching `DAIF` — the caller has already masked.
    ///
    /// **The one exception to [`Self::with`]**, and it exists for a single
    /// caller: the scheduler switch path must release the spin bit *before*
    /// `context_switch` while IRQs stay masked across the stack swap, which a
    /// closure-scoped section cannot express. `scripts/check/irq-scope.sh`
    /// refuses this call outside `src/sched/mod.rs`.
    ///
    /// The returned guard releases on `Drop`; `drop(guard)` at the point the
    /// lock must go is the explicit form.
    #[inline]
    pub fn lock_masked(&self) -> MaskedGuard<'_, T> {
        self.raw_lock();
        MaskedGuard { lock: self }
    }

    #[inline]
    fn raw_lock(&self) {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    #[inline]
    fn raw_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

/// Exclusive access from [`Mutex::lock_masked`]; releases on `Drop`.
///
/// Holds the spin bit only. It does **not** own an IRQ mask, so dropping it
/// does not unmask — the caller's `irq_save`/`irq_restore` pair still bounds
/// the masked region.
pub struct MaskedGuard<'a, T> {
    lock: &'a Mutex<T>,
}

impl<T> Deref for MaskedGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: we hold the spin bit and the caller holds the IRQ mask, so no
        // other reference to the value is live.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for MaskedGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as `deref`, and `&mut self` makes this the only borrow.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for MaskedGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.raw_unlock();
    }
}
