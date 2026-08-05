//! Cooperative scheduler (ADR-0006) + IRQ wake integration (ADR-0008).
//!
//! Voluntary yield only, fixed FIFO runqueue from `kernel-core`, idle is the
//! console loop on the bootstrap stack. IRQ handlers must not call
//! [`yield_now`], [`exit`], or [`block_current`]. They may only
//! [`wake_from_irq`]; the voluntary path drains that queue via [`poll_wakes`].
//!
//! Context switch is never nested inside [`cpu::without_irqs`]: restoring
//! another stack would leave IRQs masked with no matching restore. Bookkeeping
//! runs with IRQs hard-masked; each task re-enables on the way out of
//! [`yield_now`] / on trampoline entry.

use core::sync::atomic::{AtomicUsize, Ordering};

use kernel_core::cap::CapId;
use kernel_core::runqueue::RunQueue;
pub use kernel_core::runqueue::TaskId;
use kernel_core::wake::WakeQueue;

use crate::arch::cpu;
use crate::arch::switch::{Context, context_switch};
use crate::mm::{StackError, TaskStack};
use crate::sync::SyncCell;

/// Maximum concurrent tasks including idle.
///
/// Sized for idle + M3 demos + M4 IPC trio + M5-P/M6 agents + concurrent
/// agent peers + bringup probe margin. Slots are not reused before exit.
pub const MAX_TASKS: usize = 12;

/// Caps a task may hold (M4 local table — not shared globals).
pub const MAX_CAPS_PER_TASK: usize = 4;

/// Usable stack bytes per spawned task (plus one guard page).
const TASK_STACK_USABLE: usize = 16 * 1024;

/// Idle's fixed id — bootstrap stack, never heap-allocated.
pub const IDLE_ID: TaskId = TaskId(0);

/// IRQ → voluntary wake queue capacity (usable = N−1).
const WAKE_Q: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Empty,
    Ready,
    Running,
    /// Parked on IPC (or similar); not on the ready queue.
    Blocked,
}

struct Tcb {
    state: State,
    context: Context,
    /// `None` for idle.
    stack: Option<TaskStack>,
    /// Cleared when the trampoline starts the entry function.
    entry: Option<fn()>,
    /// Unforgeable caps this task holds (M4).
    caps: [Option<CapId>; MAX_CAPS_PER_TASK],
}

impl Tcb {
    const fn empty() -> Self {
        Self {
            state: State::Empty,
            context: Context::zeroed(),
            stack: None,
            entry: None,
            caps: [None; MAX_CAPS_PER_TASK],
        }
    }
}

struct Sched {
    tcbs: [Tcb; MAX_TASKS],
    runqueue: RunQueue<MAX_TASKS>,
    current: TaskId,
    /// Stack of a task that has exited, awaiting release by the next task to
    /// run. A task cannot free the stack its own SP points into.
    ///
    /// At most one is ever pending: the drain runs immediately after every
    /// context switch, so a second exit cannot happen before the first is
    /// collected.
    pending_free: Option<TaskStack>,
}

impl Sched {
    const fn new() -> Self {
        Self {
            tcbs: [const { Tcb::empty() }; MAX_TASKS],
            runqueue: RunQueue::new(),
            current: IDLE_ID,
            pending_free: None,
        }
    }
}

static SCHED: SyncCell<Sched> = SyncCell::new(Sched::new());
static STARTED: AtomicUsize = AtomicUsize::new(0);

/// ADR-0008: IRQ posts here; [`poll_wakes`] drains on the voluntary path.
static WAKES: WakeQueue<WAKE_Q> = WakeQueue::new();

/// Why spawn failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnError {
    NotStarted,
    Full,
    Stack(StackError),
    TooManyCaps,
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
        caps: [None; MAX_CAPS_PER_TASK],
    };
    sched.current = IDLE_ID;
    STARTED.store(1, Ordering::Release);
    cpu::irq_enable();
}

/// Create a ready task that starts at `entry` with no capabilities.
pub fn spawn(entry: fn()) -> Result<TaskId, SpawnError> {
    spawn_with_caps(entry, &[])
}

/// Create a ready task that starts at `entry` holding `caps` (M4).
pub fn spawn_with_caps(entry: fn(), caps: &[CapId]) -> Result<TaskId, SpawnError> {
    if STARTED.load(Ordering::Acquire) == 0 {
        return Err(SpawnError::NotStarted);
    }
    if caps.len() > MAX_CAPS_PER_TASK {
        return Err(SpawnError::TooManyCaps);
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

    let mut held = [None; MAX_CAPS_PER_TASK];
    for (i, &c) in caps.iter().enumerate() {
        held[i] = Some(c);
    }

    sched.tcbs[slot] = Tcb {
        state: State::Ready,
        context,
        stack: Some(stack),
        entry: Some(entry),
        caps: held,
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

/// Running task id (including idle).
#[inline]
pub fn current_task_id() -> TaskId {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        unsafe { (*SCHED.get()).current }
    })
}

/// Cap at local slot `i` for the current task, if any.
pub fn my_cap(i: usize) -> Option<CapId> {
    if i >= MAX_CAPS_PER_TASK {
        return None;
    }
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &*SCHED.get() };
        let idx = sched.current.0 as usize;
        sched.tcbs.get(idx).and_then(|t| t.caps[i])
    })
}

/// True if the current task holds `cap` in its local table.
pub fn current_holds(cap: CapId) -> bool {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &*SCHED.get() };
        let idx = sched.current.0 as usize;
        sched
            .tcbs
            .get(idx)
            .map(|t| t.caps.contains(&Some(cap)))
            .unwrap_or(false)
    })
}

/// Cooperative yield: requeue current, run the next ready task (or stay).
///
/// Never call from an IRQ handler.
pub fn yield_now() {
    poll_wakes();
    switch_with(SwitchKind::Yield);
}

/// Park the current non-idle task until [`wake_task`] / [`wake_from_irq`].
///
/// Never call from an IRQ handler or from idle.
pub fn block_current() {
    switch_with(SwitchKind::Block);
}

/// Terminate the current non-idle task and switch to the next ready (or idle).
pub fn exit() -> ! {
    switch_with(SwitchKind::Exit);
    // Idle called exit, or no one left to run.
    cpu::halt()
}

/// Make a blocked task Ready (voluntary path — e.g. IPC send).
pub fn wake_task(id: TaskId) {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &mut *SCHED.get() };
        make_ready(sched, id);
    });
}

/// Post a wake from IRQ context (ADR-0008). Never switches.
///
/// Reserved for IRQ-sourced readiness (e.g. future UART wait). Drained by
/// [`poll_wakes`] on the voluntary path only.
#[allow(dead_code)]
pub fn wake_from_irq(id: TaskId) {
    let _ = WAKES.push(id.0);
}

/// Drain the IRQ wake queue into Ready (voluntary path only).
pub fn poll_wakes() {
    while let Some(token) = WAKES.pop() {
        wake_task(TaskId(token));
    }
}

/// Wake queue drop count (full queue under IRQ pressure).
#[inline]
#[allow(dead_code)]
pub fn wake_drops() -> u32 {
    WAKES.drops()
}

fn make_ready(sched: &mut Sched, id: TaskId) {
    let idx = id.0 as usize;
    if idx >= MAX_TASKS || id == IDLE_ID {
        return;
    }
    let tcb = &mut sched.tcbs[idx];
    if tcb.state == State::Blocked {
        tcb.state = State::Ready;
        let _ = sched.runqueue.enqueue(id);
    }
}

#[derive(Clone, Copy)]
enum SwitchKind {
    Yield,
    Exit,
    Block,
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
    cpu::without_irqs(|| {
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
        count
    })
}

/// The running task's id.
#[cfg(feature = "bringup")]
pub fn current_id() -> TaskId {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        unsafe { (*SCHED.get()).current }
    })
}

/// True when the ready queue holds at least one task.
pub fn has_ready() -> bool {
    !cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        unsafe { (*SCHED.get()).runqueue.is_empty() }
    })
}

/// Return an exited task's stack, now that we are running on a different one.
///
/// # Safety
/// IRQs masked, and the caller is not the task that owned the pending stack.
unsafe fn drain_pending_free() {
    // SAFETY: IRQs masked; the borrow ends before this returns and never
    // crosses a context switch.
    let pending = unsafe { (*SCHED.get()).pending_free.take() };
    if let Some(stack) = pending {
        // SAFETY: its owner has exited and we are on another stack.
        unsafe { stack.release() };
    }
}

fn switch_with(kind: SwitchKind) {
    if STARTED.load(Ordering::Acquire) == 0 {
        return;
    }

    let daif = cpu::irq_save();

    // SAFETY: IRQs masked for schedule + switch.
    let sched = unsafe { &mut *SCHED.get() };
    let current = sched.current;
    let cur_idx = current.0 as usize;

    let requeue_current = match kind {
        SwitchKind::Yield => {
            if sched.tcbs[cur_idx].state == State::Running {
                sched.tcbs[cur_idx].state = State::Ready;
            }
            true
        }
        SwitchKind::Block => {
            if current == IDLE_ID {
                unsafe { cpu::irq_restore(daif) };
                return;
            }
            sched.tcbs[cur_idx].state = State::Blocked;
            false
        }
        SwitchKind::Exit => {
            if current == IDLE_ID {
                // Idle must not exit.
                unsafe { cpu::irq_restore(daif) };
                return;
            }
            sched.tcbs[cur_idx].state = State::Empty;
            // Not released here: still running on that stack — park for drain.
            sched.pending_free = sched.tcbs[cur_idx].stack.take();
            sched.tcbs[cur_idx].entry = None;
            sched.tcbs[cur_idx].caps = [None; MAX_CAPS_PER_TASK];
            sched.tcbs[cur_idx].context = Context::zeroed();
            false
        }
    };

    let requeue = requeue_current.then_some(current);
    let next = match sched.runqueue.after_yield(requeue) {
        Ok(Some(id)) => id,
        Ok(None) => {
            // No ready work: run idle if we are not already idle.
            if current != IDLE_ID {
                IDLE_ID
            } else {
                sched.tcbs[cur_idx].state = State::Running;
                // SAFETY: closes the section opened above.
                unsafe { cpu::irq_restore(daif) };
                return;
            }
        }
        Err(_) => {
            // Requeue failed (capacity): stay on current.
            sched.tcbs[cur_idx].state = State::Running;
            // SAFETY: closes the section opened above.
            unsafe { cpu::irq_restore(daif) };
            return;
        }
    };

    if next == current {
        sched.tcbs[cur_idx].state = State::Running;
        // SAFETY: closes the section opened above.
        unsafe { cpu::irq_restore(daif) };
        return;
    }

    let next_idx = next.0 as usize;
    sched.tcbs[next_idx].state = State::Running;
    sched.current = next;

    let prev = core::ptr::addr_of_mut!(sched.tcbs[cur_idx].context);
    let next_ctx = core::ptr::addr_of!(sched.tcbs[next_idx].context);

    // SAFETY: both contexts in static TCBs; stacks valid; IRQs masked.
    unsafe { context_switch(prev, next_ctx) };

    // Resumed as some task that was switched away from earlier, on its own
    // stack. Anything an exiting task left behind can be freed now.
    // SAFETY: IRQs still masked; we are not the task that parked it.
    unsafe { drain_pending_free() };

    // `daif` is this task's own saved mask, restored from its own frame — a
    // task always resumes at the level it left, and only first entry needs the
    // unconditional unmask in `task_trampoline`.
    // SAFETY: closes the section this task opened before it switched away.
    unsafe { cpu::irq_restore(daif) };
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
