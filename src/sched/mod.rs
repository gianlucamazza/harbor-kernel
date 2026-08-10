//! Scheduler: voluntary switches (ADR-0006) + IRQ wakes (ADR-0008) +
//! quantum preemption on the IRQ-return epilogue (ADR-0064/0068).
//!
//! Fixed FIFO runqueue from `kernel-core`, idle is the console loop on the
//! bootstrap stack. Device IRQ *handlers* still must not call
//! [`yield_now`], [`exit`], or [`block_current`] — they call
//! [`irq::wait::signal`]; the voluntary path drains that queue via
//! [`poll_wakes`]. What ADR-0068 adds is not a handler switching but the
//! EL1 vector **epilogue** (after EOI) pivoting onto the current task's own
//! stack and entering [`el1_preempt_from_irq`] when the quantum expired.
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
use kernel_core::capslots;
pub use kernel_core::runqueue::TaskId;
use kernel_core::tasks::{Decision, Switch, Tasks};

use kernel_core::density::{self, StackClass};
use kernel_core::parktime;

use crate::arch::cpu;
use crate::arch::el0::El0Session;
use crate::arch::switch::{Context, context_switch};
use crate::ipc;
use crate::irq;
use crate::mm::{StackError, TaskStack};
use crate::sync::SyncCell;
use crate::time;

/// Maximum concurrent tasks including idle.
///
/// Sized for idle + M3 demos + M4 IPC trio + M5-P/M6 agents + concurrent
/// agent peers + the manifest's two + bringup probe margin. Slots are not
/// reused before exit, so this is a boot-time total and not a high-water mark.
///
/// It went 12 → 14 when the loader landed, 14 → 16 for M8 console server +
/// product beacon, 16 → 18 for the ADR-0025 reaping oracle, then 18 → 19 for the K1 irq-wait oracle (ADR-0028),
/// 19 → 24 for K2/K3 oracles + K10 supervisor, 25 → 28 for transfer/cascade/resolve,
/// 28 → 40 for density/budget/durable residual oracles (ADR-0044..0046).
/// The four ADR-0054 peer-transfer oracle tasks landed without a bump: slot
/// reuse after exit absorbs them, and the epoch in the task identity
/// (ADR-0062) makes a leaked reference to the previous tenant refusable.
/// 40 → 42 for the ADR-0064 preemption oracle pair (spinner host + peer),
/// which are live across the window the ADR-0031 auto-reap oracle spawns in.
/// Raising it costs task stacks and page-table reserve derived from this constant.
pub const MAX_TASKS: usize = 42;

const _: () = assert!(
    MAX_TASKS <= kernel_core::irqwait::MAX_TASK_IDS,
    "irqwait pending bitmap must cover MAX_TASKS"
);

/// Caps a task may hold (M4 local table — not shared globals).
pub const MAX_CAPS_PER_TASK: usize = 4;

/// Cooperative quantum in timer ticks (ADR-0046 / K4).
pub const BUDGET_QUANTUM_TICKS: u64 = 2;

/// Slice start tick for the current task (set on switch-in).
static SLICE_START: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Mirror of "the running task is idle", written on switch-in beside
/// [`SLICE_START`] so [`preempt_switch`] reads both without opening `SCHED`.
/// Idle is current from boot, hence `true`.
static CURRENT_IS_IDLE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

/// Quantum expiries consumed by [`preempt_switch`] (ADR-0064).
static PREEMPT_SWITCHES: AtomicU32 = AtomicU32::new(0);

/// Per-slot resources. The *state* of a slot lives in [`Tasks`]; this is what
/// the kernel has to own because it cannot be modelled on a host.
struct Tcb {
    context: Context,
    /// `None` for idle.
    stack: Option<TaskStack>,
    /// Cleared when the trampoline starts the entry function.
    entry: Option<fn()>,
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
    /// Who spawned this task (ADR-0038 / K10 cascade). Idle for early boot.
    creator: TaskId,
    /// May call `SYS_RESOLVE` (ADR-0052). Default false — not ambient.
    may_resolve: bool,
}

impl Tcb {
    const fn empty() -> Self {
        Self {
            context: Context::zeroed(),
            stack: None,
            entry: None,
            el0: None,
            cancel_wait: false,
            creator: Tasks::<MAX_TASKS>::IDLE,
            may_resolve: false,
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
    let session = match sched.tcbs[to.slot()].el0.as_mut() {
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
    /// What every task may name, one row per slot (ADR-0063). The decisions
    /// over it — resolve, install, transfer, drain — are host-tested and
    /// mutated in `kernel_core::capslots`; this module contributes identity
    /// (who asks, epoch-checked liveness) and the IRQ mask.
    caps: capslots::Table<MAX_TASKS, MAX_CAPS_PER_TASK>,
}

impl Sched {
    const fn new() -> Self {
        Self {
            tasks: Tasks::new(),
            tcbs: [const { Tcb::empty() }; MAX_TASKS],
            caps: capslots::Table::new(),
        }
    }
}

static SCHED: SyncCell<Sched> = SyncCell::new(Sched::new());
static STARTED: AtomicUsize = AtomicUsize::new(0);

/// Exits that found a stack still parked from an earlier exit. See
/// [`pending_overwrites`].
static PENDING_OVERWRITES: AtomicU32 = AtomicU32::new(0);

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
        sched.tcbs[idle.slot()].el0 = Some(El0Session::new());
        publish_el0(sched, idle);
        STARTED.store(1, Ordering::Release);
    });
}

/// Create a ready task that starts at `entry` with no capabilities.
pub fn spawn(entry: fn()) -> Result<TaskId, SpawnError> {
    spawn_with_caps(entry, &[])
}

/// Thin-stack spawn (ADR-0044 / K5) — 4 KiB usable + guard.
pub fn spawn_thin(entry: fn()) -> Result<TaskId, SpawnError> {
    spawn_with_class(entry, &[], StackClass::Thin)
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
    spawn_inner(entry, slots, StackClass::Full)
}

fn spawn_with_class(entry: fn(), caps: &[CapId], class: StackClass) -> Result<TaskId, SpawnError> {
    if caps.len() > MAX_CAPS_PER_TASK {
        return Err(SpawnError::TooManyCaps);
    }
    let mut slots = [None; MAX_CAPS_PER_TASK];
    for (slot, &cap) in slots.iter_mut().zip(caps) {
        *slot = Some(cap);
    }
    spawn_inner(entry, &slots, class)
}

fn spawn_inner(
    entry: fn(),
    slots: &[Option<CapId>],
    class: StackClass,
) -> Result<TaskId, SpawnError> {
    if STARTED.load(Ordering::Acquire) == 0 {
        return Err(SpawnError::NotStarted);
    }
    if slots.len() > MAX_CAPS_PER_TASK {
        return Err(SpawnError::TooManyCaps);
    }

    let usable = density::usable_bytes(class);
    let stack = TaskStack::allocate(usable).map_err(SpawnError::Stack)?;

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
        let slot = id.slot();

        let mut context = Context::zeroed();
        context.x30 = task_trampoline as *const () as u64;
        context.sp = stack.initial_sp() as u64;

        let mut held = [None; MAX_CAPS_PER_TASK];
        held[..slots.len()].copy_from_slice(slots);
        let creator = sched.tasks.current();

        sched.tcbs[slot] = Tcb {
            context,
            stack: Some(stack),
            entry: Some(entry),
            // Created here rather than on first use: the switch publishes
            // whatever the slot holds, so a session that appears later would be
            // a pointer the last switch could not have published.
            el0: Some(El0Session::new()),
            cancel_wait: false,
            creator,
            may_resolve: false,
        };
        sched.caps.seed(slot, &held);
        // ADR-0031: SEND holds are TCB slots, not stack CapId copies.
        ipc::register_holds(&held);
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

/// Grant `SYS_RESOLVE` to `id` (ADR-0052). Trusted EL1 / creator path.
///
/// Returns `false` if `id` is not a non-empty task slot.
pub fn grant_resolve(id: TaskId) -> bool {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &mut *SCHED.get() };
        match sched.tasks.state(id) {
            Some(kernel_core::tasks::State::Empty) | None => false,
            Some(_) => {
                sched.tcbs[id.slot()].may_resolve = true;
                true
            }
        }
    })
}

/// Grant resolve to the running task (bootstrap / oracle convenience).
#[inline]
pub fn grant_resolve_current() -> bool {
    grant_resolve(current_task_id())
}

/// Whether the running task may call `SYS_RESOLVE`.
#[inline]
pub fn may_resolve_current() -> bool {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &*SCHED.get() };
        let slot = sched.tasks.current().slot();
        sched.tcbs[slot].may_resolve
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
        // A corrupted row index is a refusal rather than a panic in a syscall
        // — the table answers an out-of-range row as a row with no slots.
        sched.caps.get(sched.tasks.current().slot(), slot)
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
        let idx = sched.tasks.current().slot();
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
        sched.caps.holds(sched.tasks.current().slot(), cap)
    })
}

/// Why [`transfer_held`] refused (ADR-0037 / K3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferError {
    /// Source slot empty or out of range.
    BadFromSlot,
    /// Target not a live non-self task.
    BadToTask,
    /// Target slot already holds a cap.
    ToSlotFull,
    /// Target slot index out of range.
    ToSlotOob,
    /// Moved object is not in a transferable band (ADR-0055).
    Untransferable,
}

impl From<capslots::TransferError> for TransferError {
    fn from(e: capslots::TransferError) -> Self {
        match e {
            capslots::TransferError::BadFromSlot => Self::BadFromSlot,
            capslots::TransferError::ToSlotFull => Self::ToSlotFull,
            capslots::TransferError::ToSlotOob => Self::ToSlotOob,
            capslots::TransferError::Untransferable => Self::Untransferable,
        }
    }
}

/// Move the current task's cap at `from_slot` into `to`'s empty `to_slot` (ADR-0037).
///
/// Same-task moves are allowed (`to == current`). SEND-hold counts are unchanged
/// (same number of installs). The slot decisions live in
/// [`kernel_core::capslots`] (ADR-0063); this function contributes the ABI's
/// refusal order — bounds, then destination liveness, then the rest.
pub fn transfer_held(from_slot: usize, to: TaskId, to_slot: usize) -> Result<(), TransferError> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let sched = unsafe { &mut *SCHED.get() };
        let from = sched.tasks.current();
        capslots::Table::<MAX_TASKS, MAX_CAPS_PER_TASK>::transfer_bounds(from_slot, to_slot)?;
        if to != from {
            // `state` is the one validator: unknown slot, empty slot and a
            // stale epoch (ADR-0062) all answer "no such task".
            match sched.tasks.state(to) {
                Some(kernel_core::tasks::State::Empty) | None => {
                    return Err(TransferError::BadToTask);
                }
                Some(_) => {}
            }
        }
        sched
            .caps
            .transfer(from.slot(), from_slot, to.slot(), to_slot)
            .map_err(TransferError::from)
    })
}

/// Move current task's `from_slot` into its creator's empty `to_slot` (ADR-0041).
pub fn transfer_held_to_creator(from_slot: usize, to_slot: usize) -> Result<(), TransferError> {
    let creator = cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &*SCHED.get() };
        let idx = sched.tasks.current().slot();
        sched.tcbs[idx].creator
    });
    if creator == current_task_id() {
        return Err(TransferError::BadToTask);
    }
    transfer_held(from_slot, creator, to_slot)
}

/// Move `from_slot` into a peer named by a held task-cap (ADR-0054).
///
/// `task_cap_slot` must hold a live task-cap for the destination task. The
/// moved object must be an IPC endpoint cap: task-caps and IRQ caps are
/// refused by [`transfer_held`]'s band filter (ADR-0055).
///
/// Preemption-safe by linearization (ADR-0068 re-audit): the gaps between
/// the four masked regions carry only *names* (CapId, TaskId, slot
/// indices), never slot contents. `from_slot`'s content is read for the
/// first time inside [`transfer_held`]'s single final masked region, where
/// target liveness (the ADR-0062 epoch) and the band filter judge the
/// *current* content atomically — a preemption in a gap just means the
/// operation linearizes there, which slot-indexed authority (ADR-0017)
/// declares correct.
pub fn transfer_held_to_peer(
    from_slot: usize,
    to_slot: usize,
    task_cap_slot: usize,
) -> Result<(), TransferError> {
    let peer_cap = my_cap(task_cap_slot).ok_or(TransferError::BadToTask)?;
    let to = crate::taskcap::lookup(peer_cap).map_err(|_| TransferError::BadToTask)?;
    if to == current_task_id() {
        return Err(TransferError::BadToTask);
    }
    transfer_held(from_slot, to, to_slot)
}

/// Why [`install_cap`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// Slot out of range or already occupied.
    BadSlot,
}

/// Install `cap` into the current task's empty `slot` and register SEND holds.
pub fn install_cap(slot: usize, cap: CapId) -> Result<(), InstallError> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let sched = unsafe { &mut *SCHED.get() };
        let idx = sched.tasks.current().slot();
        sched
            .caps
            .install(idx, slot, cap)
            .map_err(|capslots::InstallError::BadSlot| InstallError::BadSlot)
    })?;
    let mut one = [None; MAX_CAPS_PER_TASK];
    one[slot] = Some(cap);
    ipc::register_holds(&one);
    Ok(())
}

static CASCADE_EVENTS: AtomicU32 = AtomicU32::new(0);

/// How many blocked children were cancelled on creator exit (ADR-0038).
#[inline]
pub fn cascade_events() -> u32 {
    CASCADE_EVENTS.load(Ordering::Relaxed)
}

/// Rotate the current task if its quantum has expired (ADR-0064).
///
/// The **voluntary-path** safe point of the ADR-0051 design (its IRQ-side
/// sibling is [`el1_preempt_pending`] + [`el1_preempt_from_irq`], ADR-0068).
/// No flag carrier: the predicate is monotone in `now` — once the tick that
/// would have raised `need_resched` fires, it stays true here until the
/// switch resets [`SLICE_START`] — so evaluating at the safe point is the
/// flag, without `time` having to know `sched` exists (layering) and
/// without a stale flag ever outliving the slice that earned it. Call
/// outside any [`cpu::without_irqs`] region, never from an IRQ handler.
pub fn preempt_switch() {
    let start = SLICE_START.load(Ordering::Relaxed);
    let idle = CURRENT_IS_IDLE.load(Ordering::Relaxed);
    if kernel_core::preempt::should_set(start, time::ticks(), BUDGET_QUANTUM_TICKS, idle) {
        PREEMPT_SWITCHES.fetch_add(1, Ordering::Relaxed);
        poll_wakes();
        switch_with(Switch::Preempt);
    }
}

/// Should the EL1 IRQ-return epilogue rotate the current task? (ADR-0068)
///
/// Called by the vector code after claim → dispatch → EOI, still in
/// exception context: reads only atomics and the tick counter — no locks,
/// no switch. `STARTED` gates the whole boot window (the loader's unmasked
/// single-threaded setup included), and the idle mirror keeps idle
/// unpreemptable. Reached by `bl` from `vectors.s`, not by an import — the
/// same deliberate asm seam as `CURRENT_EL0`.
#[unsafe(no_mangle)]
pub extern "C" fn el1_preempt_pending() -> u32 {
    if STARTED.load(Ordering::Acquire) == 0 {
        return 0;
    }
    // K8 second slice (ADR-0074): core 1 may take IRQs (wake SGI) but must
    // not pivot against the single shared scheduler. Per-core runqueues
    // retire this fence.
    if cpu::affinity() != 0 {
        return 0;
    }
    let start = SLICE_START.load(Ordering::Relaxed);
    let idle = CURRENT_IS_IDLE.load(Ordering::Relaxed);
    u32::from(kernel_core::preempt::should_set(
        start,
        time::ticks(),
        BUDGET_QUANTUM_TICKS,
        idle,
    ))
}

/// Rotate from the EL1 IRQ-return pivot (ADR-0068).
///
/// Runs on the preempted task's **own** stack after `el1_preempt_pivot`
/// moved the trap frame there and unwound the exception stack; DAIF is
/// fully masked from exception entry, and stays masked through the switch
/// and back out to the pivot's `eret` (whose SPSR restore reopens I).
///
/// Deliberately **no** [`poll_wakes`]: the wake-queue drain is a
/// single-consumer critical section on the voluntary path, and IRQ-context
/// work stays minimal (ADR-0008 spirit) — idle and `yield_now` still drain.
#[unsafe(no_mangle)]
pub extern "C" fn el1_preempt_from_irq() {
    PREEMPT_SWITCHES.fetch_add(1, Ordering::Relaxed);
    switch_with(Switch::Preempt);
}

/// Preempt rotations performed since boot (ADR-0064).
#[inline]
pub fn preempt_switches() -> u32 {
    PREEMPT_SWITCHES.load(Ordering::Relaxed)
}

/// Cooperative yield: requeue current, run the next ready task (or stay).
///
/// Never call from an IRQ handler.
pub fn yield_now() {
    poll_wakes();
    switch_with(Switch::Yield);
}

/// Park the current non-idle task until [`wake_task`] or an IRQ wait delivery.
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

/// Drain the IRQ wake queue into Ready (voluntary path only).
///
/// Also expires park deadlines (ADR-0040): cancel blocked waiters whose tick
/// deadline has passed. Never switches from IRQ — idle/`yield_now` call this.
pub fn poll_wakes() {
    irq::wait::drain(|token| wake_task(TaskId::from_raw(token)));
    poll_park_timeouts();
}

static PARK_DEADLINES: SyncCell<parktime::Table> = SyncCell::new(parktime::Table::new());

/// Arm an absolute tick deadline for a parked wait (ADR-0040).
pub fn arm_park_deadline(id: TaskId, deadline: u64) -> Result<(), parktime::ArmError> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let table = unsafe { &mut *PARK_DEADLINES.get() };
        table.arm(id, deadline)
    })
}

/// Clear a park deadline (after recv returns or cancel).
pub fn disarm_park_deadline(id: TaskId) {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let table = unsafe { &mut *PARK_DEADLINES.get() };
        table.disarm(id);
    });
}

fn poll_park_timeouts() {
    let now = time::ticks();
    let mut expired = [Tasks::<MAX_TASKS>::IDLE; parktime::MAX_ARMED];
    let n = cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let table = unsafe { &mut *PARK_DEADLINES.get() };
        table.poll(now, &mut expired)
    });
    for id in expired.iter().take(n) {
        let _ = ipc::cancel_blocked(*id);
    }
}

/// Why [`wait_for_irq`] refused to park (ADR-0028 / ADR-0030).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitIrqError {
    /// Cookie already has a waiter, this task is already armed, or table full.
    Busy,
}

/// Park the current non-idle task until the IRQ line registered with `cookie`
/// signals (ADR-0028 / K1).
///
/// Never call from an IRQ handler or from idle. Refuses to overwrite another
/// waiter's cookie (see [`kernel_core::irqwait`]).
pub fn wait_for_irq(cookie: u32) -> Result<(), WaitIrqError> {
    let me = current_task_id();
    if irq::wait::arm(cookie, me).is_err() {
        return Err(WaitIrqError::Busy);
    }
    // Lost-wakeup: IRQ may have run after arm and before park.
    if irq::wait::take_pending(me) {
        irq::wait::disarm_task(me);
        return Ok(());
    }
    block_current();
    let _ = irq::wait::take_pending(me);
    irq::wait::disarm_task(me);
    Ok(())
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
        // `state(id) == Blocked` already implies a live, in-range, non-idle
        // id: idle never blocks (model invariant) and a stale epoch answers
        // `None` (ADR-0062).
        let idx = id.slot();
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
        let idx = sched.tasks.current().slot();
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

/// Why [`supervisor_reap_blocked`] refused (ADR-0033 / K10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReapError {
    /// Target is the idle task.
    Idle,
    /// Unknown task id.
    BadId,
    /// Task is not currently [`kernel_core::tasks::State::Blocked`].
    NotBlocked,
}

static REAP_EVENTS: AtomicU32 = AtomicU32::new(0);

/// Observe a task's lifecycle state (creator/supervisor path, ADR-0033).
pub fn task_state(id: TaskId) -> Option<kernel_core::tasks::State> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let sched = unsafe { &*SCHED.get() };
        sched.tasks.state(id)
    })
}

/// Reap a **blocked** non-idle task: cancel its wait so it can exit (ADR-0033).
///
/// Product API over [`crate::ipc::cancel_blocked`]. The child must treat
/// `Cancelled` as terminal for this wait and return from its entry (trampoline
/// exits). Does **not** force-kill a Running EL0 session or destroy a remote AS.
pub fn supervisor_reap_blocked(id: TaskId) -> Result<(), ReapError> {
    if id == Tasks::<MAX_TASKS>::IDLE {
        return Err(ReapError::Idle);
    }
    match task_state(id) {
        None => Err(ReapError::BadId),
        Some(kernel_core::tasks::State::Blocked) => {
            if !ipc::cancel_blocked(id) {
                return Err(ReapError::NotBlocked);
            }
            REAP_EVENTS.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        Some(_) => Err(ReapError::NotBlocked),
    }
}

/// Successful [`supervisor_reap_blocked`] calls since boot.
#[inline]
pub fn reap_events() -> u32 {
    REAP_EVENTS.load(Ordering::Relaxed)
}

/// Wake queue drop count (full queue under IRQ pressure).
#[inline]
pub fn wake_drops() -> u32 {
    irq::wait::drops()
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
            let Some(id) = sched.tasks.live_id(slot) else {
                continue;
            };
            if let Some(stack) = tcb.stack.as_ref() {
                out[count] = StackReport {
                    id,
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
    if let Some(stack) = sched.tcbs[id.slot()].stack.take() {
        // SAFETY: its owner has exited and we are on another stack.
        unsafe { stack.release() };
    }
}

fn switch_with(kind: Switch) {
    if STARTED.load(Ordering::Acquire) == 0 {
        return;
    }

    let daif = cpu::irq_save();

    // Bookkeeping under an exclusive SCHED borrow, then drop it before ipc
    // may re-enter via `prepare_cancel_blocked` (ADR-0031).
    let (from, to, release, exit_caps) = {
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

        let exit_caps = if kind == Switch::Exit {
            // The slot is `Empty` and its stack stays attached until collected —
            // the model refuses to hand the slot out again until then.
            let caps = sched.caps.drain(from.slot());
            let slot = &mut sched.tcbs[from.slot()];
            slot.entry = None;
            slot.context = Context::zeroed();
            // The session dies with the task. Nothing scrubs what a faulting agent
            // left in it before this point — see ADR-0018's fifth reversal row.
            slot.el0 = None;
            Some(caps)
        } else {
            None
        };
        (from, to, release, exit_caps)
    };

    if let Some(caps) = exit_caps {
        ipc::release_holds_and_reap(&caps);
        // ADR-0054: task-caps naming this task become stale. The whole
        // exit→revoke window sits inside this function's irq_save…irq_restore
        // mask, so EL1 preemption (ADR-0068, DAIF-gated by construction)
        // cannot land between the model exit above and this revoke.
        let _ = crate::taskcap::revoke_task(from);
        // ADR-0038: cancel Blocked direct children of the exiting task.
        let mut kids = [Tasks::<MAX_TASKS>::IDLE; MAX_TASKS];
        let mut n = 0usize;
        {
            // SAFETY: IRQs still masked.
            let sched = unsafe { &*SCHED.get() };
            for i in 0..MAX_TASKS {
                // `live_id` is how a slot is named since ADR-0062; the exiting
                // task's slot has no live id any more, so it skips itself.
                let Some(id) = sched.tasks.live_id(i) else {
                    continue;
                };
                if id == Tasks::<MAX_TASKS>::IDLE {
                    continue;
                }
                if sched.tcbs[i].creator != from {
                    continue;
                }
                if matches!(
                    sched.tasks.state(id),
                    Some(kernel_core::tasks::State::Blocked)
                ) {
                    kids[n] = id;
                    n += 1;
                }
            }
        }
        for k in kids.iter().take(n) {
            if ipc::cancel_blocked(*k) {
                CASCADE_EVENTS.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    // SAFETY: IRQs still masked; exclusive again for context pointers.
    let sched = unsafe { &mut *SCHED.get() };

    if let Some(stranded) = release {
        // An exit that found the parked slot already taken. The model counts
        // it; this frees the stack rather than losing the pointer to it. With
        // both collection points in place it never happens.
        if let Some(stack) = sched.tcbs[stranded.slot()].stack.take() {
            // SAFETY: parked by a task that exited earlier; we are running on
            // our own stack, which is the one being parked now.
            unsafe { stack.release() };
        }
    }

    publish_el0(sched, to);
    // ADR-0046: new slice for whoever we are about to run. The idle mirror is
    // written beside it (ADR-0064) so the tick handler reads both without
    // touching `SCHED`.
    SLICE_START.store(time::ticks(), Ordering::Relaxed);
    CURRENT_IS_IDLE.store(to == Tasks::<MAX_TASKS>::IDLE, Ordering::Relaxed);

    let prev = &raw mut sched.tcbs[from.slot()].context;
    let next_ctx = &raw const sched.tcbs[to.slot()].context;

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
        let idx = sched.tasks.current().slot();
        sched.tcbs[idx].entry.take()
    });

    if let Some(entry) = entry {
        entry();
    }
    exit()
}
