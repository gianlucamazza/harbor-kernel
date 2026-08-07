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
//!
//! # What is here and what is not
//!
//! Which task runs next, what happens to the one leaving, and whose stack is
//! now safe to free are decisions, and they live in [`kernel_core::tasks`]
//! where they are host-tested. What is left here is the part that cannot be:
//! the context switch, the heap-allocated stacks and their guard pages, and the
//! interrupt mask.
//!
//! The stack of an exited task stays attached to its slot until someone
//! collects it — one place holding it, rather than a `pending_free` beside the
//! table that has to agree with it.

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use kernel_core::cap::{CapId, SlotError};
pub use kernel_core::runqueue::TaskId;
use kernel_core::tasks::{Decision, Switch, Tasks};
use kernel_core::wake::WakeQueue;

use crate::arch::cpu;
use crate::arch::el0::El0Session;
use crate::arch::switch::{Context, context_switch};
use crate::irq;
use crate::mm::{StackError, TaskStack};
use crate::sync::SyncCell;

/// Maximum concurrent tasks including idle.
///
/// Sized for idle + M3 demos + M4 IPC trio + M5-P/M6 agents + concurrent
/// agent peers + the manifest's two + bringup probe margin. Slots are not
/// reused before exit, so this is a boot-time total and not a high-water mark.
///
/// It went 12 → 14 when the loader landed, 14 → 16 for M8 console server +
/// product beacon, 16 → 18 for the ADR-0025 reaping oracle, then 18 → 19 for
/// the K1 irq-wait oracle (ADR-0028). Raising it costs task stacks and
/// page-table reserve derived from this constant.
pub const MAX_TASKS: usize = 19;

/// Caps a task may hold (M4 local table — not shared globals).
pub const MAX_CAPS_PER_TASK: usize = 4;

/// Usable stack bytes per spawned task (plus one guard page).
const TASK_STACK_USABLE: usize = 16 * 1024;

/// IRQ → voluntary wake queue capacity (usable = N−1).
const WAKE_Q: usize = 16;

/// Per-slot resources. The *state* of a slot lives in [`Tasks`]; this is what
/// the kernel has to own because it cannot be modelled on a host.
struct Tcb {
    context: Context,
    /// `None` for idle.
    stack: Option<TaskStack>,
    /// Cleared when the trampoline starts the entry function.
    entry: Option<fn()>,
    /// Unforgeable caps this task holds (M4).
    caps: [Option<CapId>; MAX_CAPS_PER_TASK],
    /// EL0 session state (ADR-0017 §1). `None` for a slot that is not a task:
    /// an empty slot, or one whose task has exited.
    ///
    /// This is what used to be nine machine-wide globals in `arch::el0`, and
    /// moving it here is what lets two agents be live at EL0 at once. The
    /// assembly still needs one linker-visible name for the *published*
    /// pointer (`CURRENT_EL0`, an `AtomicPtr` since ADR-0019), so
    /// [`publish_el0`] hands `arch` a pointer to the running task's copy on
    /// every switch.
    el0: Option<El0Session>,
    /// Set by [`cancel_blocked`]; consumed by the IPC wait path (ADR-0025).
    cancel_wait: bool,
}

impl Tcb {
    const fn empty() -> Self {
        Self {
            context: Context::zeroed(),
            stack: None,
            entry: None,
            caps: [None; MAX_CAPS_PER_TASK],
            el0: None,
            cancel_wait: false,
        }
    }
}

/// Hand `arch` the EL0 session of the task that is about to run.
///
/// Called on every switch and once at [`init`], and nowhere else. Every EL0
/// entry checks that what `arch` has published is the session the running task
/// owns ([`El0Session`] / `el0::enter`), so a switch that stops calling this is
/// a panic on the next EL0 entry rather than one agent reading another's saved
/// registers — the one "nothing" row ADR-0017 carried.
fn publish_el0(sched: &mut Sched, to: TaskId) {
    let session = match sched.tcbs[to.0 as usize].el0.as_mut() {
        Some(session) => session as *mut El0Session,
        None => core::ptr::null_mut(),
    };
    // SAFETY: the pointer names a slot of the `SCHED` static, which outlives
    // every session, and this runs with IRQs masked on the switch path — no
    // session can be live at EL0 while the scheduler is between tasks.
    unsafe { crate::arch::el0::publish(session) };
}

struct Sched {
    tasks: Tasks<MAX_TASKS>,
    tcbs: [Tcb; MAX_TASKS],
}

impl Sched {
    const fn new() -> Self {
        Self {
            tasks: Tasks::new(),
            tcbs: [const { Tcb::empty() }; MAX_TASKS],
        }
    }
}

static SCHED: SyncCell<Sched> = SyncCell::new(Sched::new());
static STARTED: AtomicUsize = AtomicUsize::new(0);

/// Exits that found a stack still parked from an earlier exit. See
/// [`pending_overwrites`].
static PENDING_OVERWRITES: AtomicU32 = AtomicU32::new(0);

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
///
/// Runs under [`cpu::without_irqs`], not an unconditional unmask: bootstrap
/// calls this after a failed `board::irq::init()` has deliberately left IRQs
/// masked, and enabling them there would arm the CPU against a GIC nothing is
/// bound to.
pub fn init() {
    cpu::without_irqs(|| {
        // SAFETY: single core; first init; IRQs masked.
        let sched = unsafe { &mut *SCHED.get() };
        sched.tasks.start();
        // Idle is a task like any other here: bootstrap runs on it, and
        // bootstrap is what enters EL0 for the demo agents.
        let idle = sched.tasks.current();
        sched.tcbs[idle.0 as usize].el0 = Some(El0Session::new());
        publish_el0(sched, idle);
        STARTED.store(1, Ordering::Release);
    });
}

/// Create a ready task that starts at `entry` with no capabilities.
pub fn spawn(entry: fn()) -> Result<TaskId, SpawnError> {
    spawn_with_caps(entry, &[])
}

/// Create a ready task holding `caps` from slot 0 upwards (M4).
///
/// Convenience over [`spawn_with_slots`] for the common case where the slots
/// are simply the order the capabilities are listed in.
pub fn spawn_with_caps(entry: fn(), caps: &[CapId]) -> Result<TaskId, SpawnError> {
    if caps.len() > MAX_CAPS_PER_TASK {
        return Err(SpawnError::TooManyCaps);
    }
    let mut slots = [None; MAX_CAPS_PER_TASK];
    for (slot, &cap) in slots.iter_mut().zip(caps) {
        *slot = Some(cap);
    }
    spawn_with_slots(entry, &slots)
}

/// Create a ready task whose capability table is `slots`, holes included.
///
/// The creator decides which slot holds what, and *may leave gaps*
/// (ADR-0017 §2). A gap is not padding: an agent that miscounts its own slots
/// is refused rather than handed whatever sits next to the one it meant, and
/// the boot oracle uses exactly that to show the refusal on the good path.
pub fn spawn_with_slots(entry: fn(), slots: &[Option<CapId>]) -> Result<TaskId, SpawnError> {
    spawn_inner(entry, slots)
}

fn spawn_inner(entry: fn(), slots: &[Option<CapId>]) -> Result<TaskId, SpawnError> {
    if STARTED.load(Ordering::Acquire) == 0 {
        return Err(SpawnError::NotStarted);
    }
    if slots.len() > MAX_CAPS_PER_TASK {
        return Err(SpawnError::TooManyCaps);
    }

    let stack = TaskStack::allocate(TASK_STACK_USABLE).map_err(SpawnError::Stack)?;

    // Restore rather than unmask: bootstrap spawns before the IRQ path is
    // necessarily live, and a caller that arrived here with IRQs masked must
    // leave with them masked.
    cpu::without_irqs(move || {
        // SAFETY: IRQs masked.
        let sched = unsafe { &mut *SCHED.get() };

        let Some(id) = sched.tasks.admit() else {
            // SAFETY: never scheduled.
            unsafe { stack.release() };
            return Err(SpawnError::Full);
        };
        let slot = id.0 as usize;

        let mut context = Context::zeroed();
        context.x30 = task_trampoline as *const () as u64;
        context.sp = stack.initial_sp() as u64;

        let mut held = [None; MAX_CAPS_PER_TASK];
        held[..slots.len()].copy_from_slice(slots);

        sched.tcbs[slot] = Tcb {
            context,
            stack: Some(stack),
            entry: Some(entry),
            caps: held,
            // Created here rather than on first use: the switch publishes
            // whatever the slot holds, so a session that appears later would be
            // a pointer the last switch could not have published.
            el0: Some(El0Session::new()),
            cancel_wait: false,
        };
        Ok(id)
    })
}

/// Running task id (including idle).
#[inline]
pub fn current_task_id() -> TaskId {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        unsafe { (*SCHED.get()).tasks.current() }
    })
}

/// Is the running task the idle task?
///
/// Asked by anything that is about to park. Idle leaving the run queue is the
/// one way to reach a core with nothing runnable, and the scheduler's own model
/// carries that as an invariant — see `model_sched.rs`. A caller that can park
/// must refuse rather than trust that idle never calls it.
#[inline]
pub fn current_is_idle() -> bool {
    current_task_id() == Tasks::<MAX_TASKS>::IDLE
}

/// Cap at local slot `i` for the current task, if any.
#[inline]
pub fn my_cap(i: usize) -> Option<CapId> {
    my_cap_slot(i).ok()
}

/// Resolve a slot against the current task's own capability table.
///
/// The whole of what an EL0 agent may name (ADR-0017 §2). The bound comes from
/// [`kernel_core::cap::from_slot`] against the array itself rather than from a
/// comparison with `MAX_CAPS_PER_TASK` written here: two statements of the same
/// length can disagree, and one of them is host-tested.
pub fn my_cap_slot(slot: usize) -> Result<CapId, SlotError> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &*SCHED.get() };
        let idx = sched.tasks.current().0 as usize;
        match sched.tcbs.get(idx) {
            Some(tcb) => kernel_core::cap::from_slot(&tcb.caps, slot),
            // The current task id always indexes the array; this arm exists so
            // a corrupted index is a refusal rather than a panic in a syscall.
            None => Err(SlotError::OutOfRange { slot, slots: 0 }),
        }
    })
}

/// Pointer to the current task's EL0 session, or null if it has none.
///
/// The scheduler's answer to *whose session is this*. `arch::el0` compares it
/// against what it has published before it will enter or resume EL0, which is
/// what makes a missing publication loud.
pub fn current_el0_session() -> *mut El0Session {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked, single core.
        let sched = unsafe { &mut *SCHED.get() };
        let idx = sched.tasks.current().0 as usize;
        match sched.tcbs[idx].el0.as_mut() {
            Some(session) => session as *mut El0Session,
            None => core::ptr::null_mut(),
        }
    })
}

/// True if the current task holds `cap` in its local table.
pub fn current_holds(cap: CapId) -> bool {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &*SCHED.get() };
        let idx = sched.tasks.current().0 as usize;
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
    switch_with(Switch::Yield);
}

/// Park the current non-idle task until [`wake_task`] / [`wake_from_irq`].
///
/// Never call from an IRQ handler or from idle.
pub fn block_current() {
    switch_with(Switch::Block);
}

/// Terminate the current non-idle task and switch to the next ready (or idle).
pub fn exit() -> ! {
    switch_with(Switch::Exit);
    // Idle called exit, or no one left to run.
    cpu::halt()
}

/// Make a blocked task Ready (voluntary path — e.g. IPC send).
pub fn wake_task(id: TaskId) {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &mut *SCHED.get() };
        sched.tasks.wake(id);
    });
}

/// Post a wake from IRQ context (ADR-0008 / ADR-0028). Never switches.
///
/// Prefer [`irq::wait::signal`] from handlers (cookie-matched). This entry
/// posts a raw task token when the caller already knows the waiter id.
#[allow(dead_code)] // raw-token path; handlers use cookie `signal`
pub fn wake_from_irq(id: TaskId) {
    irq::wait::post_task(id.0);
}

/// Drain the IRQ wake queue into Ready (voluntary path only).
pub fn poll_wakes() {
    irq::wait::drain(|token| wake_task(TaskId(token)));
    while let Some(token) = WAKES.pop() {
        wake_task(TaskId(token));
    }
}

/// Park the current non-idle task until the IRQ line registered with `cookie`
/// signals (ADR-0028 / K1).
///
/// Never call from an IRQ handler or from idle.
pub fn wait_for_irq(cookie: u32) {
    let me = current_task_id().0;
    // Arm, then check delivered before parking (lost-wakeup window).
    irq::wait::arm(cookie, me);
    if irq::wait::take_delivered() {
        irq::wait::disarm();
        return;
    }
    block_current();
    let _ = irq::wait::take_delivered();
    irq::wait::disarm();
}

/// Exits that found a stack still parked from an earlier exit.
///
/// Must stay zero: the parked stack is drained after every `context_switch` and
/// on first entry in [`task_trampoline`], so an exit can never find one. A
/// non-zero value means some path reaches `Exit` without draining. The stack is
/// still released rather than leaked, but the single-slot design no longer
/// holds — which is the interesting half.
pub fn pending_overwrites() -> u32 {
    PENDING_OVERWRITES.load(Ordering::Relaxed)
}

/// Tasks currently in [`kernel_core::tasks::State::Blocked`] (ADR-0024).
///
/// Includes intentional waiters such as the console server. Observability only
/// — nothing reclaims them.
pub fn blocked_count() -> u32 {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let sched = unsafe { &*SCHED.get() };
        sched.tasks.blocked_count()
    })
}

/// Cumulative successful entries into `Blocked` since boot (ADR-0024).
pub fn block_events() -> u32 {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let sched = unsafe { &*SCHED.get() };
        sched.tasks.block_events()
    })
}

/// Successful supervisor cancels of a blocked wait (ADR-0025).
static CANCEL_EVENTS: AtomicU32 = AtomicU32::new(0);

/// Mark a blocked task for wait cancellation and wake it (ADR-0025).
///
/// Does **not** clear the IPC waiter — the caller ([`crate::ipc::cancel_blocked`])
/// must, so `sched` does not import `ipc` (layering).
///
/// Returns `false` if `id` is idle, unknown, or not [`State::Blocked`].
pub fn prepare_cancel_blocked(id: TaskId) -> bool {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let sched = unsafe { &mut *SCHED.get() };
        if !matches!(
            sched.tasks.state(id),
            Some(kernel_core::tasks::State::Blocked)
        ) {
            return false;
        }
        let idx = id.0 as usize;
        if idx >= MAX_TASKS || id == Tasks::<MAX_TASKS>::IDLE {
            return false;
        }
        sched.tcbs[idx].cancel_wait = true;
        if !sched.tasks.wake(id) {
            return false;
        }
        CANCEL_EVENTS.fetch_add(1, Ordering::Relaxed);
        true
    })
}

/// Take and clear the current task's cancel-wait flag (ADR-0025).
pub fn take_cancel_wait() -> bool {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &mut *SCHED.get() };
        let idx = sched.tasks.current().0 as usize;
        let flag = sched
            .tcbs
            .get_mut(idx)
            .map(|t| t.cancel_wait)
            .unwrap_or(false);
        if let Some(t) = sched.tcbs.get_mut(idx) {
            t.cancel_wait = false;
        }
        flag
    })
}

/// How many times a cancel prepared successfully.
#[inline]
pub fn cancel_events() -> u32 {
    CANCEL_EVENTS.load(Ordering::Relaxed)
}

/// Wake queue drop count (full queue under IRQ pressure).
#[inline]
#[expect(dead_code, reason = "drop count for a queue that has no producer yet")]
pub fn wake_drops() -> u32 {
    irq::wait::drops() + WAKES.drops()
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
            id: Tasks::<MAX_TASKS>::IDLE,
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
        unsafe { (*SCHED.get()).tasks.current() }
    })
}

/// True when the ready queue holds at least one task.
pub fn has_ready() -> bool {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        unsafe { (*SCHED.get()).tasks.has_ready() }
    })
}

/// Release the stack of a task that has exited, if one is waiting.
///
/// The model says *which* slot; the stack itself is still attached to it, so
/// there is one owner rather than a `pending_free` beside the table.
///
/// # Safety
/// IRQs masked, and the caller is not the task that parked it.
unsafe fn collect_exited() {
    // SAFETY: IRQs masked; the borrow ends before this returns and never
    // crosses a context switch.
    let sched = unsafe { &mut *SCHED.get() };
    let Some(id) = sched.tasks.collect() else {
        return;
    };
    if let Some(stack) = sched.tcbs[id.0 as usize].stack.take() {
        // SAFETY: its owner has exited and we are on another stack.
        unsafe { stack.release() };
    }
}

fn switch_with(kind: Switch) {
    if STARTED.load(Ordering::Acquire) == 0 {
        return;
    }

    let daif = cpu::irq_save();

    // SAFETY: IRQs masked for schedule + switch.
    let sched = unsafe { &mut *SCHED.get() };

    let (from, to, release) = match sched.tasks.switch(kind) {
        Decision::Stay => {
            // SAFETY: closes the section opened above, on the path that does
            // not change stacks.
            unsafe { cpu::irq_restore(daif) };
            return;
        }
        Decision::Switch { from, to, release } => (from, to, release),
    };

    if kind == Switch::Exit {
        // The slot is `Empty` and its stack stays attached until collected —
        // the model refuses to hand the slot out again until then.
        let slot = &mut sched.tcbs[from.0 as usize];
        slot.entry = None;
        slot.caps = [None; MAX_CAPS_PER_TASK];
        slot.context = Context::zeroed();
        // The session dies with the task. Nothing scrubs what a faulting agent
        // left in it before this point — see ADR-0018's fifth reversal row.
        slot.el0 = None;
    }

    if let Some(stranded) = release {
        // An exit that found the parked slot already taken. The model counts
        // it; this frees the stack rather than losing the pointer to it. With
        // both collection points in place it never happens.
        if let Some(stack) = sched.tcbs[stranded.0 as usize].stack.take() {
            // SAFETY: parked by a task that exited earlier; we are running on
            // our own stack, which is the one being parked now.
            unsafe { stack.release() };
        }
    }

    publish_el0(sched, to);

    let prev = &raw mut sched.tcbs[from.0 as usize].context;
    let next_ctx = &raw const sched.tcbs[to.0 as usize].context;

    // SAFETY: both contexts in static TCBs; stacks valid; IRQs masked.
    unsafe { context_switch(prev, next_ctx) };

    // Resumed as some task that was switched away from earlier, on its own
    // stack. Anything an exiting task left behind can be freed now.
    // SAFETY: IRQs still masked; we are not the task that parked it.
    unsafe { collect_exited() };

    // `daif` is this task's own saved mask, restored from its own frame — a
    // task always resumes at the level it left, and only first entry needs the
    // unconditional unmask in `task_trampoline`.
    // SAFETY: closes the section this task opened before it switched away.
    unsafe { cpu::irq_restore(daif) };
}

/// First code a spawned task runs: collect, IRQs on, call entry, then exit.
///
/// This is the one place in `sched` that unmasks unconditionally, and it has to:
/// a task entered here through `context_switch`, which restores no PSTATE, so it
/// arrives with the mask [`switch_with`] was holding and has no saved `daif` of
/// its own to restore. Every other path uses [`cpu::irq_save`] /
/// [`cpu::irq_restore`].
extern "C" fn task_trampoline() -> ! {
    // Still masked from `switch_with`, and we are on our own stack — so this is
    // the same collection point the post-`context_switch` path has. Without it,
    // an exit followed by a never-yet-run task drops the parked `TaskStack`.
    // SAFETY: IRQs masked; the parked stack belongs to a task that has exited,
    // never to this one (this one has not run before).
    unsafe { collect_exited() };

    cpu::irq_enable();

    let entry = cpu::without_irqs(|| {
        // SAFETY: IRQs masked; we are the current task.
        let sched = unsafe { &mut *SCHED.get() };
        let idx = sched.tasks.current().0 as usize;
        sched.tcbs[idx].entry.take()
    });

    if let Some(entry) = entry {
        entry();
    }
    exit()
}
