//! Inter-task messages and unforgeable send/recv capabilities (M4).
//!
//! Tasks exchange fixed [`Message`] values through kernel mailboxes. A task
//! holds [`CapId`]s minted by [`create_channel`]; a send with a missing, stale,
//! or wrong-rights cap is refused and counted (M4 done-when). EL0 agents reach
//! the same mailboxes by slot index (M7 slice 2, ADR-0017 §2).
//!
//! No shared user-visible buffer: the only path for payload is this module.
//! Blocking recv parks the task via [`crate::sched::block_current`]; send wakes
//! a waiter on the voluntary path (not from IRQ).
//!
//! # What is here and what is not
//!
//! The bookkeeping — which endpoint a capability names, whether it still names
//! it, whether the rights cover the operation, where a message goes — lives in
//! [`kernel_core::ipc::Table`], where it is host-tested. That is the authority
//! surface, and it used to be checked by a single `grep` over a boot log.
//!
//! What is left here is the four things a pure table cannot do: hold the
//! global, take the interrupt mask, ask the scheduler whether the calling task
//! actually holds the capability, and wake a task the table says to wake.
//!
//! # Two ways in, one authority model
//!
//! EL1 callers pass a [`CapId`] and are checked against what the task holds.
//! EL0 agents cannot: they pass a **slot index** into their own table
//! ([`send_from_slot`], [`try_recv_from_slot`], ADR-0017 §2), and there is
//! nothing outside that array for them to name. The asymmetry is deliberate —
//! EL1 is inside the TCB, EL0 is not, and the stronger form is worth its cost
//! exactly at the boundary.
//!
//! Both paths end at the same table and the same counters. The slot translation
//! lives here rather than in `agent` because [`refused_count`] is this module's
//! to maintain: exporting a way to increment it so another module could count
//! its own refusals would put the definition of "authority violation" in two
//! places and make the number anyone's to move.
//!
//! The two checks on a send are deliberately separate.
//! [`crate::sched::current_holds`] asks *"was this capability given to you"*;
//! `Table::send` asks *"does it still name a live endpoint with this right"*.
//! A task could hold a capability whose endpoint has been revoked, and a task
//! could name a perfectly valid capability that belongs to someone else.

use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::cap::CapId;
use kernel_core::ipc::{Refusals, Table};

pub use kernel_core::ipc::{Channel, CreateError, Message, RecvError, SendError};

use crate::arch::cpu;
use crate::sched;
use crate::sync::SyncCell;

/// Mailboxes, endpoint capacity, and messages per mailbox.
///
/// Fixed and small on purpose: no allocation on the IPC path, and a full
/// mailbox is a refusal rather than unbounded growth. These are **part of the
/// EL0 ABI** (ADR-0017 §4), not implementation detail: an agent that assumes a
/// deeper mailbox breaks when it fills, and it has no way to ask.
pub const MAX_MAILBOXES: usize = 8;
pub const MAX_ENDPOINTS: usize = 16;
pub const MAILBOX_DEPTH: usize = 4;

static IPC: SyncCell<Table<MAX_MAILBOXES, MAX_ENDPOINTS, MAILBOX_DEPTH>> =
    SyncCell::new(Table::new());

/// Mirrors of [`Refusals`], published for callers that cannot take the mask.
///
/// The table counts; these make the counts readable from a print in the idle
/// loop or a bring-up path without borrowing the global. **Mirrors only** —
/// every increment goes through the table, because these are stores of the
/// table's numbers and an increment written here would be erased by the next
/// one. That was a real defect: a caller-side refusal was counted in the atomic
/// and wiped by the following successful send, and the M4 gate asserting a
/// non-zero count passed on a different refusal than the one it named.
static REFUSED_AUTHORITY: AtomicU32 = AtomicU32::new(0);
static REFUSED_FULL: AtomicU32 = AtomicU32::new(0);
static REFUSED_STATE: AtomicU32 = AtomicU32::new(0);

fn publish(refusals: Refusals) {
    REFUSED_AUTHORITY.store(refusals.authority, Ordering::Relaxed);
    REFUSED_FULL.store(refusals.full, Ordering::Relaxed);
    REFUSED_STATE.store(refusals.state, Ordering::Relaxed);
}

/// Run `f` against the table with IRQs masked, then republish the counts.
fn with_table<R>(
    f: impl FnOnce(&mut Table<MAX_MAILBOXES, MAX_ENDPOINTS, MAILBOX_DEPTH>) -> R,
) -> R {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked and one core, so this `&mut` cannot overlap
        // another. No IRQ handler touches IPC — the wake path is voluntary-only
        // by ADR-0008, which is what makes a plain `&mut` sound here.
        let table = unsafe { &mut *IPC.get() };
        let result = f(table);
        publish(table.refusals());
        result
    })
}

/// Refusals that were authority violations (M4 done-when counter).
///
/// Separate from the other two on purpose: the M4 gate asserts this is non-zero
/// to prove the forger was rejected, and a merely full mailbox used to raise the
/// same number.
#[inline]
pub fn refused_count() -> u32 {
    REFUSED_AUTHORITY.load(Ordering::Relaxed)
}

/// Sends refused for want of space. Flow control, not a violation.
#[inline]
pub fn refused_full_count() -> u32 {
    REFUSED_FULL.load(Ordering::Relaxed)
}

/// Refusals that indicate kernel bookkeeping is wrong. Should stay zero.
#[inline]
pub fn refused_state_count() -> u32 {
    REFUSED_STATE.load(Ordering::Relaxed)
}

/// Allocate a mailbox and mint send + recv capabilities.
pub fn create_channel() -> Result<Channel, CreateError> {
    with_table(|t| t.create_channel())
}

/// Send `msg` using a send capability. Wakes a blocked receiver if any.
///
/// The current task must **hold** `cap` in its local table (M4 done-when).
pub fn send(cap: CapId, msg: Message) -> Result<(), SendError> {
    if !sched::current_holds(cap) {
        // Counted *in* the table rather than beside it: the table cannot see
        // who is calling, but it owns the number, and two writers with
        // different semantics is how the count came to lie.
        with_table(|t| t.note_authority_refusal());
        return Err(SendError::BadCap);
    }
    // The wake happens outside the mask, because `wake_task` takes it again and
    // because the table deliberately reports rather than acts.
    let wake = with_table(|t| t.send(cap, msg))?;
    if let Some(id) = wake {
        sched::wake_task(id);
    }
    Ok(())
}

/// Count an authority violation detected outside this module.
///
/// `SYS_PUTC` is the one authority check that is not about a mailbox — the
/// console is named by a capability but drained by the kernel — so `agent`
/// performs it. The *number* still belongs here, for the reason the atomics
/// carry in their doc-comment: one definition of "authority violation", one
/// writer, and a counter nobody else can move.
pub fn note_authority_refusal() {
    with_table(|t| t.note_authority_refusal());
}

/// Send through the capability in `slot` of the calling task's own table.
///
/// The EL0 entry point (ADR-0017 §2). A slot out of range, an empty slot, and a
/// capability the task does not hold are the same answer to the agent and the
/// same counter: it asked for authority it was not granted.
pub fn send_from_slot(slot: usize, msg: Message) -> Result<(), SendError> {
    match sched::my_cap_slot(slot) {
        Ok(cap) => send(cap, msg),
        Err(_) => {
            with_table(|t| t.note_authority_refusal());
            Err(SendError::BadCap)
        }
    }
}

/// Count a refusal that means kernel bookkeeping asked for the impossible.
///
/// Same argument as [`note_authority_refusal`]: the caller detects it, the table
/// owns the number.
pub fn note_state_refusal() {
    with_table(|t| t.note_state_refusal());
}

/// Take a message through the capability in `slot`, **waiting** if none is
/// queued (`SYS_RECV`, ADR-0022 §1).
///
/// The whole body is [`recv`] with a slot resolved in front of it. That is the
/// point: the park sequence has a re-check inside it whose necessity is not
/// obvious, and duplicating the sequence for EL0 would be duplicating the
/// subtlety.
pub fn recv_from_slot(slot: usize) -> Result<Message, RecvError> {
    match sched::my_cap_slot(slot) {
        Ok(cap) => recv(cap),
        Err(_) => {
            with_table(|t| t.note_authority_refusal());
            Err(RecvError::BadCap)
        }
    }
}

/// Take a message through the capability in `slot` if one is queued
/// (`SYS_TRY_RECV`). Never parks.
pub fn try_recv_from_slot(slot: usize) -> Result<Message, RecvError> {
    match sched::my_cap_slot(slot) {
        Ok(cap) => try_recv(cap),
        Err(_) => {
            with_table(|t| t.note_authority_refusal());
            Err(RecvError::BadCap)
        }
    }
}

/// Take a message if one is queued. Never parks.
pub fn try_recv(cap: CapId) -> Result<Message, RecvError> {
    if !sched::current_holds(cap) {
        with_table(|t| t.note_authority_refusal());
        return Err(RecvError::BadCap);
    }
    with_table(|t| t.try_recv(cap))
}

/// Blocking recv: parks the current task until a message is available.
///
/// Must not be called from an IRQ handler. Idle **is** checked rather than
/// documented (ADR-0022 §5): an idle task that parked would leave the core with
/// nothing runnable, so it is answered [`RecvError::Empty`] and the attempt is
/// counted as a state refusal — the counter whose staying zero the boot check
/// asserts.
///
/// The mask must not be held across the park. `with_table` takes it and gives
/// it back; [`sched::block_current`] is called outside, because a `DAIF`
/// save/restore pair that spans a switch restores a value captured in an epoch
/// that has ended.
pub fn recv(cap: CapId) -> Result<Message, RecvError> {
    loop {
        match try_recv(cap) {
            Ok(msg) => return Ok(msg),
            Err(e @ (RecvError::BadCap | RecvError::Busy)) => return Err(e),
            Err(RecvError::Empty) if sched::current_is_idle() => {
                note_state_refusal();
                return Err(RecvError::Empty);
            }
            Err(RecvError::Empty) => {
                // `park` re-checks under the mask: between the `try_recv` above
                // and here the mask was dropped, so a message can have landed.
                let me = sched::current_task_id();
                match with_table(|t| t.park(cap, me))? {
                    Some(msg) => return Ok(msg),
                    None => sched::block_current(),
                }
            }
        }
    }
}
