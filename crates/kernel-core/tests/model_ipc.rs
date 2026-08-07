//! Bounded exhaustive model check of the authority core, against a reference
//! implementation written from the specification rather than from the code.
//!
//! `model_sched.rs` asserts invariants — properties that must hold in every
//! reachable state. This file asserts something stronger: for **every** sequence
//! of operations up to [`DEPTH`], the real [`Table`] and a small reference model
//! agree on *every observable_ — the exact `Ok`/`Err` variant, the message that
//! comes back, the task id handed over for waking, and all three refusal
//! counters.
//!
//! The reference is fifty lines and holds no capabilities: it is a queue length,
//! a waiter slot, and the rules as `docs` states them. Where the two disagree,
//! one of them is wrong, and which one is a question worth being asked.
//!
//! # Why this exists
//!
//! `SECURITY.md` rests every authority claim on this type, and two of its rows
//! say the check is *latent*:
//!
//! - *"Endpoint release / generation recycle: never exercised in product path;
//!   stale-handle check is latent"* — nothing in the kernel ever mints a stale
//!   `CapId`, so the generation field's whole purpose goes unexercised. Here a
//!   stale handle is in the alphabet, and it is offered at every step.
//! - Six mutants survive on the `!mbox.live` arms because no endpoint is ever
//!   released. The model shows the branch is unreachable *by exhaustion* over
//!   the API, rather than by reading the code and agreeing with it.
//!
//! # What is bounded
//!
//! `Table<2, 4, 2>` — two mailboxes, four endpoints (two channels), depth two:
//! the smallest shape where a mailbox can be full *and* a second channel exists
//! to be confused with it. Sequences of at most [`DEPTH`] operations, replayed
//! from a fresh table with no state deduplication.
//!
//! Skipped under Miri for the reason `model_sched.rs` states: interpretation is
//! ~600x slower, the walk does not finish, and this crate's only `unsafe` lives
//! in `ring.rs`, which the unit tests already cover there.
//!
//! The claim does not extend to `Table<8, 16, 4>` by proof. It extends by an
//! argument: none of the rules below mentions the number of mailboxes or the
//! depth except through `DEPTH` itself, which the model carries as a parameter.
//! The argument is written here rather than left to the reader.

use std::collections::VecDeque;

use kernel_core::cap::CapId;
use kernel_core::ipc::{Channel, Message, RecvError, SendError, Table};
use kernel_core::runqueue::TaskId;

const MAILBOXES: usize = 2;
const ENDPOINTS: usize = 4;
const DEPTH: usize = 2;

/// Longest operation sequence replayed.
const SEQ: usize = 6;

type Ipc = Table<MAILBOXES, ENDPOINTS, DEPTH>;

/// Which capability an operation names.
///
/// Five, and each one is a different way to be wrong: the right one, the one
/// with the other rights, one from another channel, one nobody minted, and one
/// that was minted for a generation that has passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Ch0Send,
    Ch0Recv,
    Ch1Send,
    /// Never minted by `create_channel`. The M4 forger in one value.
    Forged,
    /// Channel 0's send index with the previous generation — the handle the
    /// generation field exists to reject, and which no kernel path produces.
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Create,
    Send(Which),
    TryRecv(Which),
    Park(Which, u32),
}

const ALPHABET: [Op; 13] = [
    Op::Create,
    Op::Send(Which::Ch0Send),
    Op::Send(Which::Ch0Recv),
    Op::Send(Which::Ch1Send),
    Op::Send(Which::Forged),
    Op::Send(Which::Stale),
    Op::TryRecv(Which::Ch0Recv),
    Op::TryRecv(Which::Ch0Send),
    Op::TryRecv(Which::Ch1Send),
    Op::TryRecv(Which::Forged),
    Op::TryRecv(Which::Stale),
    Op::Park(Which::Ch0Recv, 1),
    Op::Park(Which::Ch0Recv, 2),
];

/// One channel as the specification describes it: a bounded queue and one
/// waiter slot. No capabilities, no endpoints — those are the real table's job,
/// and confusing the two would make this a copy of the code instead of a
/// statement about it.
#[derive(Default)]
struct RefChannel {
    queue: VecDeque<u32>,
    waiter: Option<TaskId>,
}

#[derive(Default)]
struct Reference {
    channels: Vec<RefChannel>,
    authority: u32,
    full: u32,
    state: u32,
}

/// What a capability names in the reference: a channel index and its rights,
/// or nothing at all.
enum Named {
    Send(usize),
    Recv(usize),
    Nothing,
}

impl Reference {
    /// The reference's own resolution, independent of the table's endpoints.
    fn resolve(&self, which: Which) -> Named {
        match which {
            Which::Ch0Send if !self.channels.is_empty() => Named::Send(0),
            Which::Ch0Recv if !self.channels.is_empty() => Named::Recv(0),
            Which::Ch1Send if self.channels.len() > 1 => Named::Send(1),
            // A stale handle names nothing: that is the whole point of the
            // generation field, and a model that let it through would be
            // agreeing with a bug rather than catching one.
            _ => Named::Nothing,
        }
    }
}

/// The observable result of one operation, in a form both sides can produce.
#[derive(Debug, PartialEq, Eq)]
enum Observed {
    Created(bool),
    Sent(Result<Option<TaskId>, SendError>),
    Received(Result<u32, RecvError>),
    Parked(Result<Option<u32>, RecvError>),
}

fn step_reference(model: &mut Reference, op: Op, tag: u32) -> Observed {
    match op {
        Op::Create => {
            let room = model.channels.len() < MAILBOXES;
            if room {
                model.channels.push(RefChannel::default());
            }
            Observed::Created(room)
        }
        Op::Send(which) => match model.resolve(which) {
            Named::Send(i) => {
                let ch = &mut model.channels[i];
                if ch.queue.len() == DEPTH {
                    model.full += 1;
                    Observed::Sent(Err(SendError::Full))
                } else {
                    ch.queue.push_back(tag);
                    Observed::Sent(Ok(ch.waiter.take()))
                }
            }
            _ => {
                model.authority += 1;
                Observed::Sent(Err(SendError::BadCap))
            }
        },
        Op::TryRecv(which) => match model.resolve(which) {
            Named::Recv(i) => match model.channels[i].queue.pop_front() {
                Some(tag) => Observed::Received(Ok(tag)),
                None => Observed::Received(Err(RecvError::Empty)),
            },
            _ => {
                model.authority += 1;
                Observed::Received(Err(RecvError::BadCap))
            }
        },
        Op::Park(which, me) => match model.resolve(which) {
            Named::Recv(i) => {
                let ch = &mut model.channels[i];
                if let Some(tag) = ch.queue.pop_front() {
                    return Observed::Parked(Ok(Some(tag)));
                }
                match ch.waiter {
                    Some(existing) if existing != TaskId(me) => {
                        model.state += 1;
                        Observed::Parked(Err(RecvError::Busy))
                    }
                    _ => {
                        ch.waiter = Some(TaskId(me));
                        Observed::Parked(Ok(None))
                    }
                }
            }
            _ => {
                model.authority += 1;
                Observed::Parked(Err(RecvError::BadCap))
            }
        },
    }
}

fn cap_of(channels: &[Channel], which: Which) -> CapId {
    // A capability nobody minted. Index far past the table so it cannot alias a
    // live endpoint by accident.
    const FORGED: CapId = CapId::new(0xBEEF, 0xFACE);
    match which {
        Which::Ch0Send => channels.first().map_or(FORGED, |c| c.send),
        Which::Ch0Recv => channels.first().map_or(FORGED, |c| c.recv),
        Which::Ch1Send => channels.get(1).map_or(FORGED, |c| c.send),
        Which::Forged => FORGED,
        Which::Stale => channels.first().map_or(FORGED, |c| {
            CapId::new(c.send.index(), c.send.generation().wrapping_sub(1))
        }),
    }
}

fn step_real(table: &mut Ipc, channels: &mut Vec<Channel>, op: Op, tag: u32) -> Observed {
    match op {
        Op::Create => match table.create_channel() {
            Ok(ch) => {
                channels.push(ch);
                Observed::Created(true)
            }
            Err(_) => Observed::Created(false),
        },
        Op::Send(which) => {
            let msg = Message {
                tag,
                a: u64::from(tag),
                b: 0,
            };
            Observed::Sent(table.send(cap_of(channels, which), msg))
        }
        Op::TryRecv(which) => {
            Observed::Received(table.try_recv(cap_of(channels, which)).map(|m| m.tag))
        }
        Op::Park(which, me) => Observed::Parked(
            table
                .park(cap_of(channels, which), TaskId(me))
                .map(|m| m.map(|m| m.tag)),
        ),
    }
}

/// Replay one sequence against both, returning the step and what disagreed.
fn replay(seq: &[Op]) -> Option<(usize, String)> {
    let mut table = Ipc::new();
    let mut channels = Vec::new();
    let mut model = Reference::default();

    for (i, &op) in seq.iter().enumerate() {
        let tag = i as u32 + 1;
        let expected = step_reference(&mut model, op, tag);
        let actual = step_real(&mut table, &mut channels, op, tag);
        if expected != actual {
            return Some((
                i + 1,
                format!("{op:?}: reference says {expected:?}, table says {actual:?}"),
            ));
        }
        let r = table.refusals();
        if (r.authority, r.full, r.state) != (model.authority, model.full, model.state) {
            return Some((
                i + 1,
                format!(
                    "{op:?}: counters disagree — reference \
                     (authority {}, full {}, state {}), table (authority {}, full {}, state {})",
                    model.authority, model.full, model.state, r.authority, r.full, r.state
                ),
            ));
        }
    }

    // Conservation, checked once the sequence is over: everything the table
    // accepted is either still queued or was handed back, and nothing was
    // invented. Draining is destructive, which is why it happens last.
    for (i, ch) in channels.iter().enumerate() {
        let mut drained = Vec::new();
        while let Ok(msg) = table.try_recv(ch.recv) {
            drained.push(msg.tag);
        }
        let expected: Vec<u32> = model.channels[i].queue.iter().copied().collect();
        if drained != expected {
            return Some((
                seq.len(),
                format!("channel {i} drained {drained:?}, reference had {expected:?}"),
            ));
        }
    }

    None
}

#[cfg_attr(
    miri,
    ignore = "interpreted, the walk takes hours; this crate's only unsafe is ring.rs, which the unit tests already put under Miri"
)]
#[test]
fn the_table_agrees_with_the_specification_on_every_sequence() {
    let alphabet = ALPHABET.len();
    let mut sequences = 0u64;
    let mut buf = [Op::Create; SEQ];

    for len in 0..=SEQ {
        let total = (alphabet as u64).pow(len as u32);
        for n in 0..total {
            let mut rest = n;
            for slot in buf.iter_mut().take(len) {
                *slot = ALPHABET[(rest % alphabet as u64) as usize];
                rest /= alphabet as u64;
            }
            let seq = &buf[..len];
            sequences += 1;
            if let Some((step, why)) = replay(seq) {
                panic!(
                    "table and specification diverge at step {step}\n  {why}\n\
                     counter-example ({len} ops): {seq:?}",
                );
            }
        }
    }

    println!(
        "model_ipc: {sequences} sequences over {alphabet} operations, seq ≤ {SEQ}, \
         Table<{MAILBOXES}, {ENDPOINTS}, {DEPTH}> — table matches the specification"
    );
}

/// A stale handle is offered at every step of every sequence above. This states
/// the outcome as a property in its own right, because it is the one
/// `SECURITY.md` calls latent — and a property buried inside a general
/// agreement check is a property nobody can cite.
#[test]
fn a_stale_or_forged_handle_is_never_accepted() {
    let mut table = Ipc::new();
    let ch = table.create_channel().expect("first channel");
    let stale = CapId::new(ch.send.index(), ch.send.generation().wrapping_sub(1));
    let forged = CapId::new(0xBEEF, 0xFACE);
    let msg = Message { tag: 1, a: 1, b: 0 };

    for bad in [stale, forged] {
        assert_eq!(table.send(bad, msg), Err(SendError::BadCap));
        assert_eq!(table.try_recv(bad), Err(RecvError::BadCap));
        assert_eq!(table.park(bad, TaskId(1)), Err(RecvError::BadCap));
    }

    // And the live one still works, so the refusals above are about the handle
    // and not about a table that has stopped functioning.
    assert_eq!(table.send(ch.send, msg), Ok(None));
    assert_eq!(table.try_recv(ch.recv).map(|m| m.tag), Ok(1));
    assert_eq!(table.refusals().authority, 6);
}

/// The search must reach the states it claims to cover, or it is green and
/// empty.
#[cfg_attr(
    miri,
    ignore = "interpreted, the walk takes hours; this crate's only unsafe is ring.rs, which the unit tests already put under Miri"
)]
#[test]
fn the_search_reaches_full_mailboxes_and_contended_waiters() {
    let alphabet = ALPHABET.len();
    let mut buf = [Op::Create; SEQ];
    let mut saw_full = false;
    let mut saw_busy = false;
    let mut saw_wake = false;
    let mut saw_two_channels = false;

    for len in 0..=SEQ {
        let total = (alphabet as u64).pow(len as u32);
        for n in 0..total {
            let mut rest = n;
            for slot in buf.iter_mut().take(len) {
                *slot = ALPHABET[(rest % alphabet as u64) as usize];
                rest /= alphabet as u64;
            }
            let mut table = Ipc::new();
            let mut channels = Vec::new();
            for (i, &op) in buf[..len].iter().enumerate() {
                match step_real(&mut table, &mut channels, op, i as u32 + 1) {
                    Observed::Sent(Err(SendError::Full)) => saw_full = true,
                    Observed::Sent(Ok(Some(_))) => saw_wake = true,
                    Observed::Parked(Err(RecvError::Busy)) => saw_busy = true,
                    _ => {}
                }
            }
            saw_two_channels |= channels.len() == 2;
        }
    }

    assert!(saw_full, "no sequence ever filled a mailbox");
    assert!(saw_wake, "no send ever handed back a waiter to wake");
    assert!(
        saw_busy,
        "no sequence ever contended the single waiter slot — the Busy path is untested"
    );
    assert!(saw_two_channels, "no sequence ever created both channels");
}
