//! Cooperative scheduler (ADR-0006).
//!
//! Voluntary yield only, fixed FIFO runqueue from `kernel-core`, idle is the
//! console loop on the bootstrap stack. IRQ handlers must not call
//! [`yield_now`] or [`exit`].
//!
//! Context switch is never nested inside [`cpu::without_irqs`]: restoring
//! another stack would leave IRQs masked with no matching restore. Bookkeeping
//! runs with IRQs hard-masked; each task re-enables on the way out of
//! [`yield_now`] / on trampoline entry.

use core::sync::atomic::{AtomicUsize, Ordering};

use kernel_core::runqueue::RunQueue;
pub use kernel_core::runqueue::TaskId;

use crate::arch::cpu;
use crate::arch::switch::{Context, context_switch};
use crate::mm::{StackError, TaskStack};
use crate::sync::SyncCell;

/// Maximum concurrent tasks including idle.
pub const MAX_TASKS: usize = 4;

/// Usable stack bytes per spawned task (plus one guard page).
const TASK_STACK_USABLE: usize = 16 * 1024;

/// Idle's fixed id — bootstrap stack, never heap-allocated.
pub const IDLE_ID: TaskId = TaskId(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Empty,
    Ready,
    Running,
}

struct Tcb {
    state: State,
    context: Context,
    /// `None` for idle.
    stack: Option<TaskStack>,
    /// Cleared when the trampoline starts the entry function.
    entry: Option<fn()>,
}

impl Tcb {
    const fn empty() -> Self {
        Self {
            state: State::Empty,
            context: Context::zeroed(),
            stack: None,
            entry: None,
        }
    }
}

struct Sched {
    tcbs: [Tcb; MAX_TASKS],
    runqueue: RunQueue<MAX_TASKS>,
    current: TaskId,
}

impl Sched {
    const fn new() -> Self {
        Self {
            tcbs: [const { Tcb::empty() }; MAX_TASKS],
            runqueue: RunQueue::new(),
            current: IDLE_ID,
        }
    }
}

static SCHED: SyncCell<Sched> = SyncCell::new(Sched::new());
static STARTED: AtomicUsize = AtomicUsize::new(0);

/// Why spawn failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnError {
    NotStarted,
    Full,
    Stack(StackError),
}

/// Claim idle as the running task on the bootstrap stack.
pub fn init() {
    cpu::irq_disable();
    // SAFETY: single core; first init; IRQs masked.
    let sched = unsafe { &mut *SCHED.get() };
    sched.tcbs[IDLE_ID.0 as usize] = Tcb {
        state: State::Running,
        context: Context::zeroed(),
        stack: None,
        entry: None,
    };
    sched.current = IDLE_ID;
    STARTED.store(1, Ordering::Release);
    cpu::irq_enable();
}

/// Create a ready task that starts at `entry`.
pub fn spawn(entry: fn()) -> Result<TaskId, SpawnError> {
    if STARTED.load(Ordering::Acquire) == 0 {
        return Err(SpawnError::NotStarted);
    }

    let stack = TaskStack::allocate(TASK_STACK_USABLE).map_err(SpawnError::Stack)?;

    cpu::irq_disable();
    // SAFETY: IRQs masked.
    let sched = unsafe { &mut *SCHED.get() };

    let slot = match sched.tcbs.iter().position(|t| t.state == State::Empty) {
        Some(slot) => slot,
        None => {
            cpu::irq_enable();
            // SAFETY: never scheduled.
            unsafe { stack.release() };
            return Err(SpawnError::Full);
        }
    };

    let id = TaskId(slot as u32);
    let mut context = Context::zeroed();
    context.x30 = task_trampoline as *const () as u64;
    context.sp = stack.initial_sp() as u64;

    sched.tcbs[slot] = Tcb {
        state: State::Ready,
        context,
        stack: Some(stack),
        entry: Some(entry),
    };

    if sched.runqueue.enqueue(id).is_err() {
        let mut tcb = core::mem::replace(&mut sched.tcbs[slot], Tcb::empty());
        if let Some(owned) = tcb.stack.take() {
            // SAFETY: never scheduled.
            unsafe { owned.release() };
        }
        cpu::irq_enable();
        return Err(SpawnError::Full);
    }

    cpu::irq_enable();
    Ok(id)
}

/// Cooperative yield: requeue current, run the next ready task (or stay).
///
/// Never call from an IRQ handler.
pub fn yield_now() {
    switch_with(true);
}

/// Terminate the current non-idle task and switch to the next ready (or idle).
pub fn exit() -> ! {
    switch_with(false);
    // Idle called exit, or no one left to run.
    cpu::halt()
}

/// One task's stack geometry, as reported to a bring-up probe.
#[cfg(feature = "bringup")]
#[derive(Clone, Copy)]
pub struct StackReport {
    pub id: TaskId,
    /// Unmapped guard page, `[low, high)`.
    pub guard: (u64, u64),
    /// Usable stack, `[low, high)`.
    pub stack: (u64, u64),
}

#[cfg(feature = "bringup")]
impl StackReport {
    pub const fn empty() -> Self {
        Self {
            id: IDLE_ID,
            guard: (0, 0),
            stack: (0, 0),
        }
    }
}

/// Geometry of every live task stack, for bring-up probes.
///
/// Fills `out` and returns the written prefix. Idle is skipped: it runs on the
/// `link.ld` bootstrap stack, whose guard is a different mechanism (never
/// mapped, rather than unmapped at spawn).
///
/// The probe needs every range, not just its own, because the claim under test
/// is that an overflow lands in its own guard *instead of* a peer's stack.
#[cfg(feature = "bringup")]
pub fn stack_map(out: &mut [StackReport]) -> usize {
    cpu::irq_disable();
    // SAFETY: IRQs masked; single core.
    let sched = unsafe { &*SCHED.get() };
    let mut count = 0;
    for (slot, tcb) in sched.tcbs.iter().enumerate() {
        if count == out.len() {
            break;
        }
        if let Some(stack) = tcb.stack.as_ref() {
            out[count] = StackReport {
                id: TaskId(slot as u32),
                guard: stack.guard_range(),
                stack: stack.stack_range(),
            };
            count += 1;
        }
    }
    cpu::irq_enable();
    count
}

/// The running task's id.
#[cfg(feature = "bringup")]
pub fn current_id() -> TaskId {
    cpu::irq_disable();
    // SAFETY: IRQs masked; single core.
    let id = unsafe { (*SCHED.get()).current };
    cpu::irq_enable();
    id
}

/// True when the ready queue holds at least one task.
pub fn has_ready() -> bool {
    cpu::irq_disable();
    // SAFETY: IRQs masked.
    let empty = unsafe { (*SCHED.get()).runqueue.is_empty() };
    cpu::irq_enable();
    !empty
}

fn switch_with(requeue_current: bool) {
    if STARTED.load(Ordering::Acquire) == 0 {
        return;
    }

    cpu::irq_disable();

    // SAFETY: IRQs masked for schedule + switch.
    let sched = unsafe { &mut *SCHED.get() };
    let current = sched.current;
    let cur_idx = current.0 as usize;

    if !requeue_current {
        if current == IDLE_ID {
            // Idle must not exit.
            cpu::irq_enable();
            return;
        }
        sched.tcbs[cur_idx].state = State::Empty;
        if let Some(stack) = sched.tcbs[cur_idx].stack.take() {
            // SAFETY: this task will not resume; switch goes elsewhere.
            unsafe { stack.release() };
        }
        sched.tcbs[cur_idx].entry = None;
        sched.tcbs[cur_idx].context = Context::zeroed();
    } else if sched.tcbs[cur_idx].state == State::Running {
        sched.tcbs[cur_idx].state = State::Ready;
    }

    let requeue = requeue_current.then_some(current);
    let next = match sched.runqueue.after_yield(requeue) {
        Ok(Some(id)) => id,
        Ok(None) => {
            // No ready work: run idle if we are not already idle.
            if current != IDLE_ID {
                IDLE_ID
            } else {
                sched.tcbs[cur_idx].state = State::Running;
                cpu::irq_enable();
                return;
            }
        }
        Err(_) => {
            // Requeue failed (capacity): stay on current.
            sched.tcbs[cur_idx].state = State::Running;
            cpu::irq_enable();
            return;
        }
    };

    if next == current {
        sched.tcbs[cur_idx].state = State::Running;
        cpu::irq_enable();
        return;
    }

    let next_idx = next.0 as usize;
    sched.tcbs[next_idx].state = State::Running;
    sched.current = next;

    let prev = core::ptr::addr_of_mut!(sched.tcbs[cur_idx].context);
    let next_ctx = core::ptr::addr_of!(sched.tcbs[next_idx].context);

    // SAFETY: both contexts in static TCBs; stacks valid; IRQs masked.
    unsafe { context_switch(prev, next_ctx) };

    // Resumed here as whatever task was switched to (or back).
    cpu::irq_enable();
}

/// First code a spawned task runs: IRQs on, call entry, then exit.
extern "C" fn task_trampoline() -> ! {
    cpu::irq_enable();

    let entry = {
        cpu::irq_disable();
        // SAFETY: IRQs masked; we are the current task.
        let sched = unsafe { &mut *SCHED.get() };
        let idx = sched.current.0 as usize;
        let entry = sched.tcbs[idx].entry.take();
        cpu::irq_enable();
        entry
    };

    if let Some(entry) = entry {
        entry();
    }
    exit()
}
