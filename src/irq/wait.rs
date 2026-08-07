//! IRQ wait port (ADR-0028 / K1): arm on the voluntary path, signal from IRQ.
//!
//! Handlers must not import `sched`. They call [`signal`]; [`crate::sched::poll_wakes`]
//! drains the queue into Ready. One armed waiter at a time (v1).

use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::wake::WakeQueue;

/// Sentinel: no armed task / cookie.
const NONE: u32 = u32::MAX;

/// Cookie the current waiter is waiting for, or [`NONE`].
static ARM_COOKIE: AtomicU32 = AtomicU32::new(NONE);

/// Task token (scheduler id) armed for [`ARM_COOKIE`], or [`NONE`].
static ARM_TASK: AtomicU32 = AtomicU32::new(NONE);

/// Set by [`signal`] when it matches an armed waiter; cleared by the waiter.
static DELIVERED: AtomicU32 = AtomicU32::new(0);

/// SPSC: IRQ producer → voluntary consumer (same rules as ADR-0008).
static QUEUE: WakeQueue<16> = WakeQueue::new();

/// How many `signal` calls found no matching waiter (observability).
static SIGNAL_NO_WAITER: AtomicU32 = AtomicU32::new(0);

/// Arm `task` to be woken when `cookie` is signalled.
///
/// Last arm wins (v1 single waiter). Call from the voluntary path only.
pub fn arm(cookie: u32, task: u32) {
    ARM_TASK.store(task, Ordering::Release);
    ARM_COOKIE.store(cookie, Ordering::Release);
}

/// Disarm without waking. Safe if nothing is armed.
pub fn disarm() {
    ARM_COOKIE.store(NONE, Ordering::Release);
    ARM_TASK.store(NONE, Ordering::Release);
}

/// IRQ path: if a waiter is armed for `cookie`, post a wake and mark delivered.
///
/// Never switches. Safe to call with no waiter (counts and returns).
pub fn signal(cookie: u32) {
    let armed = ARM_COOKIE.load(Ordering::Acquire);
    if armed != cookie {
        return;
    }
    let task = ARM_TASK.load(Ordering::Acquire);
    if task == NONE {
        SIGNAL_NO_WAITER.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // Disarm first so a level-triggered storm does not flood the queue.
    ARM_COOKIE.store(NONE, Ordering::Release);
    DELIVERED.store(1, Ordering::Release);
    if !QUEUE.push(task) {
        // Queue full: delivered flag still lets the waiter return if it has not
        // blocked yet; if already blocked, cancel_blocked / timeout is K2 debt.
        SIGNAL_NO_WAITER.fetch_add(1, Ordering::Relaxed);
    }
}

/// Consume the delivered flag. Returns true if an IRQ already posted for us.
pub fn take_delivered() -> bool {
    DELIVERED.swap(0, Ordering::AcqRel) != 0
}

/// Drain IRQ wake tokens into `f` (voluntary path only).
pub fn drain(mut f: impl FnMut(u32)) {
    while let Some(token) = QUEUE.pop() {
        f(token);
    }
}

/// Direct post of a task token (used by [`crate::sched::wake_from_irq`]).
#[allow(dead_code)] // public IRQ port; call sites grow with device waits
pub fn post_task(task: u32) {
    let _ = QUEUE.push(task);
}

/// Wakes dropped because the IRQ queue was full.
pub fn drops() -> u32 {
    QUEUE.drops()
}

/// Signals that found no usable waiter (or failed to push).
#[allow(dead_code)] // observability for drivers / later gates
pub fn signal_no_waiter() -> u32 {
    SIGNAL_NO_WAITER.load(Ordering::Relaxed)
}
