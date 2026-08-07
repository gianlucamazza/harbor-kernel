//! IRQ wait port (ADR-0028 / K1): arm on the voluntary path, signal from IRQ.
//!
//! Handlers must not import `sched`. They call [`signal`]; [`crate::sched::poll_wakes`]
//! drains the queue into Ready. Wait table rules live in
//! [`kernel_core::irqwait`] (host-tested): one waiter per cookie, no overwrite.

use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::irqwait::{ArmError, WaitTable};
use kernel_core::wake::WakeQueue;

use crate::arch::cpu;
use crate::sync::SyncCell;

/// SPSC: IRQ producer → voluntary consumer (ADR-0008). Sized for all tasks.
const Q: usize = 32;

static TABLE: SyncCell<WaitTable> = SyncCell::new(WaitTable::new());
static QUEUE: WakeQueue<Q> = WakeQueue::new();
static SIGNAL_IDLE: AtomicU32 = AtomicU32::new(0);

/// Arm `task` for `cookie`. Returns an error instead of overwriting another waiter.
pub fn arm(cookie: u32, task: u32) -> Result<(), ArmError> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let table = unsafe { &mut *TABLE.get() };
        table.arm(cookie, task)
    })
}

/// Drop any arm for `task`.
pub fn disarm_task(task: u32) {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let table = unsafe { &mut *TABLE.get() };
        table.disarm_task(task);
    });
}

/// IRQ path: match cookie, mark pending, enqueue wake token.
///
/// Never switches. If the queue is full, pending remains set so the waiter can
/// still observe delivery via [`take_pending`] after a spurious resume path;
/// [`crate::sched::poll_wakes`] also re-checks pending for ready promotion when
/// drops are non-zero is unnecessary if we scan — waiters always check pending
/// before and after block.
pub fn signal(cookie: u32) {
    let task = cpu::without_irqs(|| {
        // SAFETY: IRQs masked for table mutation; handler may run nested only
        // with DAIF set, so this is the sole writer for the duration.
        let table = unsafe { &mut *TABLE.get() };
        table.signal(cookie)
    });
    let Some(task) = task else {
        SIGNAL_IDLE.fetch_add(1, Ordering::Relaxed);
        return;
    };
    // Pending bit is already set inside `signal`. Queue is the fast path to Ready.
    enqueue(task);
}

/// Consume pending for `task`. True if an IRQ already posted.
pub fn take_pending(task: u32) -> bool {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let table = unsafe { &mut *TABLE.get() };
        table.take_pending(task)
    })
}

/// Drain IRQ wake tokens into `f` (voluntary path only).
pub fn drain(mut f: impl FnMut(u32)) {
    while let Some(token) = QUEUE.pop() {
        f(token);
    }
}

fn enqueue(task: u32) {
    // Capacity is ≥ task count; a full queue under a correct arm is a fault class.
    if !QUEUE.push(task) {
        SIGNAL_IDLE.fetch_add(1, Ordering::Relaxed);
    }
}

/// Wakes dropped because the IRQ queue was full.
pub fn drops() -> u32 {
    QUEUE.drops()
}

/// Signals with no waiter, or queue-push failures after a match.
pub fn signal_idle() -> u32 {
    SIGNAL_IDLE.load(Ordering::Relaxed)
}
