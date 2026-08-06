//! Inter-task messages and unforgeable send/recv capabilities (M4).
//!
//! Tasks exchange fixed [`Message`] values through kernel mailboxes. A task
//! holds [`CapId`]s minted by [`create_channel`]; a send with a missing, stale,
//! or wrong-rights cap is refused and counted (M4 done-when).
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
/// mailbox is a refusal rather than unbounded growth. These are a de-facto ABI
/// — an agent that assumes a deeper mailbox breaks when it fills — and no ADR
/// states them. The EL0 capability ABI, when it is written, will have to.
const MAX_MAILBOXES: usize = 8;
const MAX_ENDPOINTS: usize = 16;
const MAILBOX_DEPTH: usize = 4;

static IPC: SyncCell<Table<MAX_MAILBOXES, MAX_ENDPOINTS, MAILBOX_DEPTH>> =
    SyncCell::new(Table::new());

/// Mirrors of [`Refusals`], published for callers that cannot take the mask.
///
/// The table counts; these make the counts readable from a print in the idle
/// loop or a bring-up path without borrowing the global.
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
        // Counted here rather than by the table: the table cannot see who is
        // calling, and this is an authority violation like any other.
        REFUSED_AUTHORITY.fetch_add(1, Ordering::Relaxed);
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

/// Take a message if one is queued. Never parks.
pub fn try_recv(cap: CapId) -> Result<Message, RecvError> {
    if !sched::current_holds(cap) {
        REFUSED_AUTHORITY.fetch_add(1, Ordering::Relaxed);
        return Err(RecvError::BadCap);
    }
    with_table(|t| t.try_recv(cap))
}

/// Blocking recv: parks the current task until a message is available.
///
/// Must not be called from idle or an IRQ handler.
pub fn recv(cap: CapId) -> Result<Message, RecvError> {
    loop {
        match try_recv(cap) {
            Ok(msg) => return Ok(msg),
            Err(e @ (RecvError::BadCap | RecvError::Busy)) => return Err(e),
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
