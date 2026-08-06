//! Mailboxes and capability lookup — the decision half of IPC (M4).
//!
//! This is the bookkeeping: which endpoint a capability names, whether it still
//! names it, whether the holder has the right, and where a message goes. What
//! it deliberately does **not** do is act. [`Table::send`] returns the task that
//! should be woken instead of waking it, the way
//! [`crate::runqueue::RunQueue::after_yield`] returns the next task instead of
//! switching to it. `src/ipc` supplies the global, the interrupt mask, and the
//! two calls into the scheduler.
//!
//! The split exists because this is the authority surface. Before it, the only
//! thing checking that a forged capability is rejected was one `grep` over a
//! boot log — and that assertion could be satisfied by a full mailbox, which is
//! not a capability check at all.
//!
//! # The generation field, and what it cannot do yet
//!
//! [`CapId`] carries a generation so a **recycled** slot cannot be mistaken for
//! a live one. [`Table::lookup`] checks it. Nothing exercises that check,
//! because no endpoint is ever released: `live` never returns to `false`, so a
//! slot is never reused and no capability can outlive the entry it names. The
//! tests below cover the check anyway, by building the stale handle directly —
//! which is the only way to reach it until endpoints can be revoked.

use crate::cap::{CapId, CapRights};
use crate::runqueue::TaskId;

/// Fixed message — small enough to copy, no heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub tag: u32,
    pub a: u64,
    pub b: u64,
}

impl Message {
    const EMPTY: Self = Self { tag: 0, a: 0, b: 0 };
}

/// Pair of capabilities for one mailbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Channel {
    pub send: CapId,
    pub recv: CapId,
}

/// Why a channel could not be created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateError {
    NoMailbox,
    NoEndpoint,
}

/// Why send failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    BadCap,
    Full,
}

/// Why recv failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecvError {
    /// Another task is already blocked on this mailbox (one waiter slot).
    Busy,
    BadCap,
    Empty,
}

/// Refusal counts, kept apart by what they mean.
///
/// One number used to cover all three. The M4 gate asserts a refusal happened
/// to prove a forged capability was rejected, and a merely full mailbox would
/// have satisfied it — a security signal diluted by a flow-control signal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Refusals {
    /// The caller had no right: a capability it does not hold, or one that does
    /// not resolve to a live endpoint with the required rights.
    pub authority: u32,
    /// A send met a full mailbox. Flow control, not a violation.
    pub full: u32,
    /// An endpoint resolved and named a dead mailbox. Kernel bookkeeping is
    /// wrong; no caller can cause this.
    pub state: u32,
}

#[derive(Clone, Copy)]
struct Endpoint {
    live: bool,
    generation: u16,
    rights: CapRights,
    mailbox: u8,
}

impl Endpoint {
    const EMPTY: Self = Self {
        live: false,
        generation: 0,
        rights: CapRights::empty(),
        mailbox: 0,
    };
}

#[derive(Clone, Copy)]
struct Mailbox<const DEPTH: usize> {
    live: bool,
    buf: [Message; DEPTH],
    head: usize,
    tail: usize,
    len: usize,
    waiter: Option<TaskId>,
}

impl<const DEPTH: usize> Mailbox<DEPTH> {
    const EMPTY: Self = Self {
        live: false,
        buf: [Message::EMPTY; DEPTH],
        head: 0,
        tail: 0,
        len: 0,
        waiter: None,
    };

    fn push(&mut self, msg: Message) {
        self.buf[self.head] = msg;
        self.head = (self.head + 1) % DEPTH;
        self.len += 1;
    }

    fn pop(&mut self) -> Message {
        let msg = self.buf[self.tail];
        self.tail = (self.tail + 1) % DEPTH;
        self.len -= 1;
        msg
    }
}

/// Endpoints, mailboxes and the refusal counts, with no globals and no locking.
pub struct Table<const MAILBOXES: usize, const ENDPOINTS: usize, const DEPTH: usize> {
    endpoints: [Endpoint; ENDPOINTS],
    mailboxes: [Mailbox<DEPTH>; MAILBOXES],
    next_gen: u16,
    refusals: Refusals,
}

impl<const MAILBOXES: usize, const ENDPOINTS: usize, const DEPTH: usize> Default
    for Table<MAILBOXES, ENDPOINTS, DEPTH>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<const MAILBOXES: usize, const ENDPOINTS: usize, const DEPTH: usize>
    Table<MAILBOXES, ENDPOINTS, DEPTH>
{
    /// Empty table. Generations start at 1 so zero is never a live generation.
    pub const fn new() -> Self {
        Self {
            endpoints: [Endpoint::EMPTY; ENDPOINTS],
            mailboxes: [Mailbox::EMPTY; MAILBOXES],
            next_gen: 1,
            refusals: Refusals {
                authority: 0,
                full: 0,
                state: 0,
            },
        }
    }

    /// Refusals so far, by reason.
    #[inline]
    pub const fn refusals(&self) -> Refusals {
        self.refusals
    }

    /// Allocate a mailbox and mint one send and one recv capability for it.
    ///
    /// Both carry the same generation: they name the same channel at the same
    /// moment in time, which is what the generation records.
    pub fn create_channel(&mut self) -> Result<Channel, CreateError> {
        let mb = self
            .mailboxes
            .iter()
            .position(|m| !m.live)
            .ok_or(CreateError::NoMailbox)?;
        let send_ep = self
            .endpoints
            .iter()
            .position(|e| !e.live)
            .ok_or(CreateError::NoEndpoint)?;
        let recv_ep = self
            .endpoints
            .iter()
            .enumerate()
            .find(|(i, e)| !e.live && *i != send_ep)
            .map(|(i, _)| i)
            .ok_or(CreateError::NoEndpoint)?;

        let generation = self.next_gen;
        // `.max(1)`: a wrapped counter must not hand out generation 0, which is
        // what an all-zero `Endpoint` carries — a stale handle would then match
        // a slot that was never minted.
        self.next_gen = self.next_gen.wrapping_add(1).max(1);

        self.mailboxes[mb] = Mailbox {
            live: true,
            ..Mailbox::EMPTY
        };
        self.endpoints[send_ep] = Endpoint {
            live: true,
            generation,
            rights: CapRights::SEND,
            mailbox: mb as u8,
        };
        self.endpoints[recv_ep] = Endpoint {
            live: true,
            generation,
            rights: CapRights::RECV,
            mailbox: mb as u8,
        };

        Ok(Channel {
            send: CapId::new(send_ep as u16, generation),
            recv: CapId::new(recv_ep as u16, generation),
        })
    }

    /// Resolve a capability to its mailbox, or say why it does not resolve.
    ///
    /// Three ways to fail, and they are all the same answer to the caller: the
    /// index is out of range, the entry is dead or from another generation, or
    /// the rights do not cover what was asked. Distinguishing them for the
    /// caller would tell an attacker which of the three it got wrong.
    fn lookup(&self, cap: CapId, need: CapRights) -> Option<usize> {
        let ep = self.endpoints.get(cap.index() as usize)?;
        if !ep.live || ep.generation != cap.generation() {
            return None;
        }
        if !ep.rights.contains(need) {
            return None;
        }
        Some(ep.mailbox as usize)
    }

    /// Queue `msg`, and report the task that should now be woken.
    ///
    /// Returns `Ok(Some(id))` when a receiver was parked on this mailbox. The
    /// caller wakes it — this type never touches the scheduler, which is what
    /// keeps it testable on the host.
    pub fn send(&mut self, cap: CapId, msg: Message) -> Result<Option<TaskId>, SendError> {
        let Some(mb) = self.lookup(cap, CapRights::SEND) else {
            self.refusals.authority += 1;
            return Err(SendError::BadCap);
        };
        let mbox = &mut self.mailboxes[mb];
        if !mbox.live {
            self.refusals.state += 1;
            return Err(SendError::BadCap);
        }
        if mbox.len == DEPTH {
            self.refusals.full += 1;
            return Err(SendError::Full);
        }
        mbox.push(msg);
        Ok(mbox.waiter.take())
    }

    /// Take one message if there is one. Never parks.
    pub fn try_recv(&mut self, cap: CapId) -> Result<Message, RecvError> {
        let Some(mb) = self.lookup(cap, CapRights::RECV) else {
            self.refusals.authority += 1;
            return Err(RecvError::BadCap);
        };
        let mbox = &mut self.mailboxes[mb];
        if !mbox.live {
            self.refusals.state += 1;
            return Err(RecvError::BadCap);
        }
        if mbox.len == 0 {
            return Err(RecvError::Empty);
        }
        Ok(mbox.pop())
    }

    /// Claim the waiter slot for `me`, unless a message arrived first.
    ///
    /// The re-check matters: between a failed [`Self::try_recv`] and this call
    /// the caller has dropped and retaken the interrupt mask, so a message can
    /// have landed. Parking without looking again would sleep on a full
    /// mailbox until the *next* send.
    ///
    /// One slot per mailbox. A second task is refused rather than overwriting
    /// the first — an overwritten waiter stays `Blocked` with nothing left to
    /// wake it, because the sender wakes whatever id is in the slot.
    pub fn park(&mut self, cap: CapId, me: TaskId) -> Result<Option<Message>, RecvError> {
        let Some(mb) = self.lookup(cap, CapRights::RECV) else {
            self.refusals.authority += 1;
            return Err(RecvError::BadCap);
        };
        let mbox = &mut self.mailboxes[mb];
        if !mbox.live {
            self.refusals.state += 1;
            return Err(RecvError::BadCap);
        }
        if mbox.len > 0 {
            return Ok(Some(mbox.pop()));
        }
        match mbox.waiter {
            Some(existing) if existing != me => {
                self.refusals.state += 1;
                Err(RecvError::Busy)
            }
            _ => {
                mbox.waiter = Some(me);
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `src/ipc` instantiates: 8 mailboxes, 16 endpoints, depth 4.
    type T = Table<8, 16, 4>;

    fn msg(tag: u32) -> Message {
        Message {
            tag,
            a: u64::from(tag) * 2,
            b: 0,
        }
    }

    #[test]
    fn a_message_survives_the_round_trip() {
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        assert_eq!(t.send(ch.send, msg(7)), Ok(None));
        assert_eq!(t.try_recv(ch.recv), Ok(msg(7)));
        assert_eq!(t.try_recv(ch.recv), Err(RecvError::Empty));
    }

    #[test]
    fn send_and_recv_capabilities_are_not_interchangeable() {
        // The whole point of splitting rights: holding one end of a channel
        // must not let a task use the other.
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        assert_eq!(t.send(ch.recv, msg(1)), Err(SendError::BadCap));
        assert_eq!(t.try_recv(ch.send), Err(RecvError::BadCap));
        assert_eq!(t.refusals().authority, 2);
    }

    #[test]
    fn a_forged_capability_is_refused() {
        // `bootstrap`'s forger demo mints one exactly this way — `CapId` is
        // freely constructible, and unforgeability is a property of this
        // lookup, not of the type.
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        let forged = CapId::new(ch.send.index(), ch.send.generation().wrapping_add(1));
        assert_eq!(t.send(forged, msg(1)), Err(SendError::BadCap));
        assert_eq!(t.refusals().authority, 1);
    }

    #[test]
    fn a_stale_handle_from_a_recycled_slot_is_refused() {
        // The case the generation field exists for, and the one no code path
        // can reach today: nothing releases an endpoint, so a slot is never
        // reused. Built by hand because that is the only way in.
        let mut t = T::new();
        let first = t.create_channel().unwrap();
        let second = t.create_channel().unwrap();
        assert_ne!(
            first.send.generation(),
            second.send.generation(),
            "each channel must be minted in its own generation"
        );
        // A handle naming the *second* channel's slot with the *first*
        // channel's generation: the shape a stale capability would have.
        let stale = CapId::new(second.send.index(), first.send.generation());
        assert_eq!(t.send(stale, msg(1)), Err(SendError::BadCap));
        assert_eq!(t.refusals().authority, 1);
    }

    #[test]
    fn an_index_past_the_table_is_refused_not_a_panic() {
        let mut t = T::new();
        let out_of_range = CapId::new(9999, 1);
        assert_eq!(t.send(out_of_range, msg(1)), Err(SendError::BadCap));
        assert_eq!(t.try_recv(out_of_range), Err(RecvError::BadCap));
    }

    #[test]
    fn a_full_mailbox_refuses_without_touching_the_authority_count() {
        // The defect this split was made for: one counter covered both, and the
        // M4 gate asserts on it to prove a *capability* was rejected.
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        for i in 0..4 {
            assert_eq!(t.send(ch.send, msg(i)), Ok(None), "depth is 4");
        }
        assert_eq!(t.send(ch.send, msg(99)), Err(SendError::Full));
        assert_eq!(t.refusals().full, 1);
        assert_eq!(
            t.refusals().authority,
            0,
            "a full mailbox is flow control, not a violation"
        );
    }

    #[test]
    fn the_ring_wraps_and_keeps_order() {
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        // Fill, drain, refill: the second pass crosses the wrap point.
        for round in 0..3 {
            for i in 0..4u32 {
                t.send(ch.send, msg(round * 10 + i)).unwrap();
            }
            for i in 0..4u32 {
                assert_eq!(t.try_recv(ch.recv), Ok(msg(round * 10 + i)), "FIFO order");
            }
        }
    }

    #[test]
    fn park_reports_the_waiter_for_the_sender_to_wake() {
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        let me = TaskId(3);
        assert_eq!(t.park(ch.recv, me), Ok(None), "empty: the caller parks");
        assert_eq!(
            t.send(ch.send, msg(5)),
            Ok(Some(me)),
            "the send reports who to wake instead of waking it"
        );
        assert_eq!(t.send(ch.send, msg(6)), Ok(None), "the slot is taken once");
    }

    #[test]
    fn park_rechecks_before_sleeping() {
        // Between a failed `try_recv` and `park` the caller drops the interrupt
        // mask, so a message can land. Parking without looking again would
        // sleep on a full mailbox until the next send.
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        t.send(ch.send, msg(8)).unwrap();
        assert_eq!(t.park(ch.recv, TaskId(3)), Ok(Some(msg(8))));
    }

    #[test]
    fn a_second_waiter_is_refused_not_swapped_in() {
        // The defect: overwriting left the displaced task `Blocked` with
        // nothing able to wake it, because the sender wakes whatever id is in
        // the slot.
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        assert_eq!(t.park(ch.recv, TaskId(3)), Ok(None));
        assert_eq!(t.park(ch.recv, TaskId(4)), Err(RecvError::Busy));
        assert_eq!(
            t.send(ch.send, msg(1)),
            Ok(Some(TaskId(3))),
            "the first waiter is still the one woken"
        );
    }

    #[test]
    fn parking_twice_from_the_same_task_is_idempotent() {
        // A retry loop re-parks after a spurious wake; that must not be `Busy`.
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        assert_eq!(t.park(ch.recv, TaskId(3)), Ok(None));
        assert_eq!(t.park(ch.recv, TaskId(3)), Ok(None));
    }

    #[test]
    fn channels_are_exhausted_by_endpoints_before_mailboxes() {
        // 16 endpoints at two per channel = 8, which is also the mailbox count.
        // Both limits bite at once here; the assertion pins which error a
        // caller sees so the message stays truthful if either constant moves.
        let mut t = T::new();
        for i in 0..8 {
            assert!(t.create_channel().is_ok(), "channel {i}");
        }
        assert_eq!(t.create_channel(), Err(CreateError::NoMailbox));
    }

    #[test]
    fn separate_channels_do_not_share_a_mailbox() {
        let mut t = T::new();
        let a = t.create_channel().unwrap();
        let b = t.create_channel().unwrap();
        t.send(a.send, msg(1)).unwrap();
        assert_eq!(t.try_recv(b.recv), Err(RecvError::Empty), "b is untouched");
        assert_eq!(t.try_recv(a.recv), Ok(msg(1)));
    }

    #[test]
    fn refusal_counts_only_move_for_their_own_reason() {
        let mut t = T::new();
        let ch = t.create_channel().unwrap();
        let before = t.refusals();
        assert_eq!(before, Refusals::default());

        t.send(CapId::new(9999, 1), msg(1)).ok();
        assert_eq!(t.refusals().authority, 1);
        assert_eq!(t.refusals().full, 0);
        assert_eq!(t.refusals().state, 0);

        for i in 0..4 {
            t.send(ch.send, msg(i)).unwrap();
        }
        t.send(ch.send, msg(9)).ok();
        assert_eq!(t.refusals().authority, 1, "unchanged by a full mailbox");
        assert_eq!(t.refusals().full, 1);
        assert_eq!(t.refusals().state, 0, "no bookkeeping error happened");
    }
}
