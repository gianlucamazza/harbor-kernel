//! Inter-task messages and unforgeable send/recv capabilities (M4).
//!
//! Tasks exchange fixed [`Message`] values through kernel mailboxes. A task
//! holds [`CapId`]s minted by [`create_channel`]; a send with a missing,
//! stale, or wrong-rights cap is refused and counted (M4 done-when).
//!
//! No shared user-visible buffer: the only path for payload is this module.
//! Blocking recv parks the task via [`crate::sched::block_current`]; send wakes
//! a waiter on the voluntary path (not from IRQ).

use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::cap::{CapId, CapRights};

use crate::arch::cpu;
use crate::sched::{self, TaskId};
use crate::sync::SyncCell;

/// Fixed message — small enough to copy, no heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub tag: u32,
    pub a: u64,
    pub b: u64,
}

const MAX_MAILBOXES: usize = 8;
const MAILBOX_DEPTH: usize = 4;
const MAX_ENDPOINTS: usize = 16;

#[derive(Clone, Copy)]
struct Endpoint {
    live: bool,
    generation: u16,
    rights: CapRights,
    mailbox: u8,
}

struct Mailbox {
    live: bool,
    /// Ring of pending messages.
    buf: [Message; MAILBOX_DEPTH],
    head: usize,
    tail: usize,
    len: usize,
    /// Task blocked in recv, if any.
    waiter: Option<TaskId>,
}

impl Mailbox {
    const fn empty() -> Self {
        Self {
            live: false,
            buf: [Message {
                tag: 0,
                a: 0,
                b: 0,
            }; MAILBOX_DEPTH],
            head: 0,
            tail: 0,
            len: 0,
            waiter: None,
        }
    }
}

struct Ipc {
    endpoints: [Endpoint; MAX_ENDPOINTS],
    mailboxes: [Mailbox; MAX_MAILBOXES],
    next_gen: u16,
}

impl Ipc {
    const fn new() -> Self {
        Self {
            endpoints: [Endpoint {
                live: false,
                generation: 0,
                rights: CapRights::empty(),
                mailbox: 0,
            }; MAX_ENDPOINTS],
            mailboxes: [const { Mailbox::empty() }; MAX_MAILBOXES],
            next_gen: 1,
        }
    }
}

static IPC: SyncCell<Ipc> = SyncCell::new(Ipc::new());

/// Send / recv refused (bad cap, full, etc.).
static REFUSED: AtomicU32 = AtomicU32::new(0);

/// How many IPC operations were refused (M4 done-when counter).
#[inline]
pub fn refused_count() -> u32 {
    REFUSED.load(Ordering::Relaxed)
}

fn bump_refused() {
    REFUSED.fetch_add(1, Ordering::Relaxed);
}

/// Why a channel could not be created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateError {
    NoMailbox,
    NoEndpoint,
}

/// Pair of capabilities for one mailbox.
#[derive(Clone, Copy, Debug)]
pub struct Channel {
    pub send: CapId,
    pub recv: CapId,
}

/// Allocate a mailbox and mint send + recv capabilities.
pub fn create_channel() -> Result<Channel, CreateError> {
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked; single core.
        let ipc = unsafe { &mut *IPC.get() };

        let mb = ipc
            .mailboxes
            .iter()
            .position(|m| !m.live)
            .ok_or(CreateError::NoMailbox)?;
        let send_ep = ipc
            .endpoints
            .iter()
            .position(|e| !e.live)
            .ok_or(CreateError::NoEndpoint)?;
        let recv_ep = ipc
            .endpoints
            .iter()
            .enumerate()
            .find(|(i, e)| !e.live && *i != send_ep)
            .map(|(i, _)| i)
            .ok_or(CreateError::NoEndpoint)?;

        let generation = ipc.next_gen;
        ipc.next_gen = ipc.next_gen.wrapping_add(1).max(1);

        ipc.mailboxes[mb] = Mailbox {
            live: true,
            ..Mailbox::empty()
        };

        ipc.endpoints[send_ep] = Endpoint {
            live: true,
            generation,
            rights: CapRights::SEND,
            mailbox: mb as u8,
        };
        ipc.endpoints[recv_ep] = Endpoint {
            live: true,
            generation,
            rights: CapRights::RECV,
            mailbox: mb as u8,
        };

        Ok(Channel {
            send: CapId::new(send_ep as u16, generation),
            recv: CapId::new(recv_ep as u16, generation),
        })
    })
}

/// Why send failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    BadCap,
    Full,
}

/// Why recv failed (non-blocking try).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecvError {
    BadCap,
    Empty,
}

fn lookup_endpoint(ipc: &Ipc, cap: CapId, need: CapRights) -> Result<(u16, u8), ()> {
    let idx = cap.index() as usize;
    let ep = ipc.endpoints.get(idx).ok_or(())?;
    if !ep.live || ep.generation != cap.generation() {
        return Err(());
    }
    if !ep.rights.contains(need) {
        return Err(());
    }
    Ok((cap.index(), ep.mailbox))
}

/// Send `msg` using a send capability. Wakes a blocked receiver if any.
///
/// The current task must **hold** `cap` in its local table (M4 done-when).
pub fn send(cap: CapId, msg: Message) -> Result<(), SendError> {
    if !sched::current_holds(cap) {
        bump_refused();
        return Err(SendError::BadCap);
    }
    let wake = cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let ipc = unsafe { &mut *IPC.get() };
        let mailbox = match lookup_endpoint(ipc, cap, CapRights::SEND) {
            Ok((_, mb)) => mb as usize,
            Err(()) => {
                bump_refused();
                return Err(SendError::BadCap);
            }
        };
        let mbox = &mut ipc.mailboxes[mailbox];
        if !mbox.live {
            bump_refused();
            return Err(SendError::BadCap);
        }
        if mbox.len == MAILBOX_DEPTH {
            bump_refused();
            return Err(SendError::Full);
        }
        mbox.buf[mbox.head] = msg;
        mbox.head = (mbox.head + 1) % MAILBOX_DEPTH;
        mbox.len += 1;
        Ok(mbox.waiter.take())
    })?;

    if let Some(id) = wake {
        // Voluntary path: make the waiter Ready directly (not via IRQ queue).
        sched::wake_task(id);
    }
    Ok(())
}

/// Non-blocking recv.
pub fn try_recv(cap: CapId) -> Result<Message, RecvError> {
    if !sched::current_holds(cap) {
        bump_refused();
        return Err(RecvError::BadCap);
    }
    cpu::without_irqs(|| {
        // SAFETY: IRQs masked.
        let ipc = unsafe { &mut *IPC.get() };
        let mailbox = match lookup_endpoint(ipc, cap, CapRights::RECV) {
            Ok((_, mb)) => mb as usize,
            Err(()) => {
                bump_refused();
                return Err(RecvError::BadCap);
            }
        };
        let mbox = &mut ipc.mailboxes[mailbox];
        if !mbox.live {
            bump_refused();
            return Err(RecvError::BadCap);
        }
        if mbox.len == 0 {
            return Err(RecvError::Empty);
        }
        let msg = mbox.buf[mbox.tail];
        mbox.tail = (mbox.tail + 1) % MAILBOX_DEPTH;
        mbox.len -= 1;
        Ok(msg)
    })
}

/// Blocking recv: parks the current task until a message is available.
///
/// Must not be called from idle or an IRQ handler.
pub fn recv(cap: CapId) -> Result<Message, RecvError> {
    loop {
        match try_recv(cap) {
            Ok(msg) => return Ok(msg),
            Err(RecvError::BadCap) => return Err(RecvError::BadCap),
            Err(RecvError::Empty) => {
                let parked = cpu::without_irqs(|| {
                    // SAFETY: IRQs masked.
                    let ipc = unsafe { &mut *IPC.get() };
                    let mailbox = match lookup_endpoint(ipc, cap, CapRights::RECV) {
                        Ok((_, mb)) => mb as usize,
                        Err(()) => {
                            bump_refused();
                            return Err(RecvError::BadCap);
                        }
                    };
                    // Double-check empty under the same lock as waiter install.
                    let mbox = &mut ipc.mailboxes[mailbox];
                    if mbox.len > 0 {
                        let msg = mbox.buf[mbox.tail];
                        mbox.tail = (mbox.tail + 1) % MAILBOX_DEPTH;
                        mbox.len -= 1;
                        return Ok(Some(msg));
                    }
                    let me = sched::current_task_id();
                    mbox.waiter = Some(me);
                    Ok(None)
                })?;

                if let Some(msg) = parked {
                    return Ok(msg);
                }
                sched::block_current();
            }
        }
    }
}
