//! Syscall reply mapping (ADR-0060) — pure, host-tested.
//!
//! Which subsystem answer becomes which [`Status`], which counter bumps, and
//! which reply registers get written. This is the security-visible half of the
//! syscall ABI: `SECURITY.md`'s table names the calls, `doc-claims` checks the
//! set, and the tests here check the adjective neither can — the semantics.
//!
//! `src/agent` converts each subsystem `Result` into the outcome enum
//! (mechanical, one arm per variant), calls the mapper, and applies the
//! [`Reply`]. The lookups that need kernel state (slot → CapId, holds, the
//! resolve grant) stay kernel-side; the *decision after the answer* lives
//! here, where mutation testing can reach it.

use crate::syscall::Status;

/// Why a call refused, as the stable `x1` detail code (ADR-0061).
///
/// `x0` carries the class; this is the reason. Codes are ABI: renumbering is a
/// successor ADR's job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum RefusalDetail {
    BadCap = 1,
    UnknownDest = 2,
    BadFromSlot = 3,
    BadToTask = 4,
    ToSlotFull = 5,
    ToSlotOob = 6,
    Untransferable = 7,
    NoGrant = 8,
    BadNameLen = 9,
    Missing = 10,
    BadSlot = 11,
    NotIrqCap = 12,
}

impl RefusalDetail {
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self as u64
    }
}

/// Which `SessionStats` fields bump, and by how much (applied saturating).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatDelta {
    pub sends: u32,
    pub recvs: u32,
    pub recv_empties: u32,
    pub wait_irqs: u32,
    pub authority_refusals: u32,
}

/// One syscall's reply: status for `x0`, optional `x1..x3` payload, counters.
///
/// `payload` is `Some` only when the call delivers data (a received message).
/// On a refusal it is `None` — the reply is the status plus, per ADR-0061,
/// the `detail` code in `x1`; `x2`/`x3` are never touched outside a payload.
/// The tests below pin both halves per outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: Status,
    pub payload: Option<[u64; 3]>,
    /// `x1` on a refusal (ADR-0061); `None` everywhere else.
    pub detail: Option<RefusalDetail>,
    pub delta: StatDelta,
}

impl Reply {
    /// An authority refusal: `Authority`, its `x1` detail, exactly one count.
    const fn refused(detail: RefusalDetail) -> Self {
        Self {
            status: Status::Authority,
            payload: None,
            detail: Some(detail),
            delta: StatDelta {
                sends: 0,
                recvs: 0,
                recv_empties: 0,
                wait_irqs: 0,
                authority_refusals: 1,
            },
        }
    }

    /// A status that is neither success nor an authority event: no payload,
    /// no counter. `Busy`, `Full`, `Cancelled` are flow/state signals and
    /// deliberately stay off the counter the boot oracle asserts exactly.
    const fn bare(status: Status) -> Self {
        Self {
            status,
            payload: None,
            detail: None,
            delta: StatDelta {
                sends: 0,
                recvs: 0,
                recv_empties: 0,
                wait_irqs: 0,
                authority_refusals: 0,
            },
        }
    }
}

/// What the IPC layer answered a receive (blocking, non-blocking, or timed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecvOutcome {
    Got { tag: u32, a: u64, b: u64 },
    Empty,
    Busy,
    Cancelled,
    BadCap,
}

/// Shared by `SYS_RECV`, `SYS_TRY_RECV` and `SYS_RECV_TIMEOUT`: only the
/// waiting differs, never the reply.
pub fn recv(outcome: RecvOutcome) -> Reply {
    match outcome {
        RecvOutcome::Got { tag, a, b } => Reply {
            status: Status::Ok,
            payload: Some([u64::from(tag), a, b]),
            detail: None,
            delta: StatDelta {
                recvs: 1,
                ..StatDelta::default()
            },
        },
        RecvOutcome::Empty => Reply {
            status: Status::Empty,
            payload: None,
            detail: None,
            delta: StatDelta {
                recv_empties: 1,
                ..StatDelta::default()
            },
        },
        // Someone else already waits there. The agent holds what it named, so
        // this never touches the authority counter (ADR-0022 §4).
        RecvOutcome::Busy => Reply::bare(Status::Busy),
        RecvOutcome::Cancelled => Reply::bare(Status::Cancelled),
        RecvOutcome::BadCap => Reply::refused(RefusalDetail::BadCap),
    }
}

/// What the IPC layer answered a send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    /// Flow control, not a violation.
    Full,
    Refused,
}

pub fn send(outcome: SendOutcome) -> Reply {
    match outcome {
        SendOutcome::Sent => Reply {
            status: Status::Ok,
            payload: None,
            detail: None,
            delta: StatDelta {
                sends: 1,
                ..StatDelta::default()
            },
        },
        SendOutcome::Full => Reply::bare(Status::Full),
        SendOutcome::Refused => Reply::refused(RefusalDetail::BadCap),
    }
}

/// What the wait-on-IRQ path answered (ADR-0028/0030).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitIrqOutcome {
    Woken,
    /// Cookie or task already armed — state, not authority (ADR-0028).
    Busy,
    /// Slot empty/OOB or not held.
    BadCap,
    /// Held, but not an IRQ notification cap.
    NotIrqCap,
}

pub fn wait_irq(outcome: WaitIrqOutcome) -> Reply {
    match outcome {
        WaitIrqOutcome::Woken => Reply {
            status: Status::Ok,
            payload: None,
            detail: None,
            delta: StatDelta {
                wait_irqs: 1,
                ..StatDelta::default()
            },
        },
        WaitIrqOutcome::Busy => Reply::bare(Status::Busy),
        WaitIrqOutcome::BadCap => Reply::refused(RefusalDetail::BadCap),
        WaitIrqOutcome::NotIrqCap => Reply::refused(RefusalDetail::NotIrqCap),
    }
}

/// What the transfer path answered (ADR-0041/0054/0055).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferOutcome {
    Moved,
    /// `x2` named a dest this kernel does not decode.
    UnknownDest,
    /// Source slot empty or out of range.
    BadFromSlot,
    /// Target dead, unknown, self, or named by a stale/absent task-cap.
    BadToTask,
    /// Destination slot occupied.
    ToSlotFull,
    /// Destination slot index out of range.
    ToSlotOob,
    /// Moved object not an endpoint cap (ADR-0055).
    Untransferable,
}

pub fn transfer(outcome: TransferOutcome) -> Reply {
    match outcome {
        TransferOutcome::Moved => Reply::bare(Status::Ok),
        TransferOutcome::UnknownDest => Reply::refused(RefusalDetail::UnknownDest),
        TransferOutcome::BadFromSlot => Reply::refused(RefusalDetail::BadFromSlot),
        TransferOutcome::BadToTask => Reply::refused(RefusalDetail::BadToTask),
        TransferOutcome::ToSlotFull => Reply::refused(RefusalDetail::ToSlotFull),
        TransferOutcome::ToSlotOob => Reply::refused(RefusalDetail::ToSlotOob),
        TransferOutcome::Untransferable => Reply::refused(RefusalDetail::Untransferable),
    }
}

/// What the resolve path answered (ADR-0039/0052).
///
/// Four distinct causes, four distinct `x1` details (ADR-0061).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveOutcome {
    Installed,
    NoGrant,
    BadNameLen,
    Missing,
    BadSlot,
}

pub fn resolve(outcome: ResolveOutcome) -> Reply {
    match outcome {
        ResolveOutcome::Installed => Reply::bare(Status::Ok),
        ResolveOutcome::NoGrant => Reply::refused(RefusalDetail::NoGrant),
        ResolveOutcome::BadNameLen => Reply::refused(RefusalDetail::BadNameLen),
        ResolveOutcome::Missing => Reply::refused(RefusalDetail::Missing),
        ResolveOutcome::BadSlot => Reply::refused(RefusalDetail::BadSlot),
    }
}

/// Unpack a `SYS_RESOLVE` name from its registers: `len` in 1..=8, bytes
/// little-endian in `packed`. `None` is the `BadNameLen` refusal.
pub fn unpack_name(len: usize, packed: u64) -> Option<[u8; 8]> {
    if !(1..=8).contains(&len) {
        return None;
    }
    let mut name = [0u8; 8];
    for (i, b) in name.iter_mut().enumerate().take(len) {
        *b = ((packed >> (8 * i)) & 0xff) as u8;
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_authority(r: Reply, detail: RefusalDetail) {
        assert_eq!(r.status, Status::Authority);
        assert_eq!(r.payload, None);
        assert_eq!(r.detail, Some(detail), "x1 detail (ADR-0061)");
        assert_eq!(
            r.delta,
            StatDelta {
                authority_refusals: 1,
                ..StatDelta::default()
            }
        );
    }

    #[test]
    fn recv_ok_carries_payload_and_exactly_one_recv_count() {
        let r = recv(RecvOutcome::Got {
            tag: 7,
            a: 42,
            b: 9,
        });
        assert_eq!(r.status, Status::Ok);
        assert_eq!(r.payload, Some([7, 42, 9]));
        assert_eq!(
            r.delta,
            StatDelta {
                recvs: 1,
                ..StatDelta::default()
            }
        );
    }

    #[test]
    fn recv_refusals_write_no_payload() {
        // A payload arrives only on Ok; every non-Ok outcome carries at most
        // the ADR-0061 detail in x1 and leaves x2/x3 alone.
        for outcome in [
            RecvOutcome::Empty,
            RecvOutcome::Busy,
            RecvOutcome::Cancelled,
            RecvOutcome::BadCap,
        ] {
            assert_eq!(recv(outcome).payload, None, "{outcome:?}");
        }
    }

    #[test]
    fn recv_statuses_and_counters_match_the_abi() {
        assert_eq!(recv(RecvOutcome::Empty).status, Status::Empty);
        assert_eq!(recv(RecvOutcome::Empty).delta.recv_empties, 1);
        assert_eq!(recv(RecvOutcome::Busy).status, Status::Busy);
        assert_eq!(recv(RecvOutcome::Busy).delta, StatDelta::default());
        assert_eq!(recv(RecvOutcome::Cancelled).status, Status::Cancelled);
        assert_eq!(recv(RecvOutcome::Cancelled).delta, StatDelta::default());
        only_authority(recv(RecvOutcome::BadCap), RefusalDetail::BadCap);
    }

    #[test]
    fn send_maps_full_off_the_authority_counter() {
        assert_eq!(send(SendOutcome::Sent).status, Status::Ok);
        assert_eq!(send(SendOutcome::Sent).delta.sends, 1);
        let full = send(SendOutcome::Full);
        assert_eq!(full.status, Status::Full);
        assert_eq!(full.delta, StatDelta::default());
        only_authority(send(SendOutcome::Refused), RefusalDetail::BadCap);
    }

    #[test]
    fn wait_irq_busy_is_state_not_authority() {
        assert_eq!(wait_irq(WaitIrqOutcome::Woken).status, Status::Ok);
        assert_eq!(wait_irq(WaitIrqOutcome::Woken).delta.wait_irqs, 1);
        let busy = wait_irq(WaitIrqOutcome::Busy);
        assert_eq!(busy.status, Status::Busy);
        assert_eq!(busy.delta, StatDelta::default());
        only_authority(wait_irq(WaitIrqOutcome::BadCap), RefusalDetail::BadCap);
        only_authority(
            wait_irq(WaitIrqOutcome::NotIrqCap),
            RefusalDetail::NotIrqCap,
        );
    }

    #[test]
    fn transfer_ok_counts_nothing_and_every_refusal_counts_once() {
        let ok = transfer(TransferOutcome::Moved);
        assert_eq!(ok.status, Status::Ok);
        assert_eq!(ok.delta, StatDelta::default());
        assert_eq!(ok.detail, None);
        for (outcome, detail) in [
            (TransferOutcome::UnknownDest, RefusalDetail::UnknownDest),
            (TransferOutcome::BadFromSlot, RefusalDetail::BadFromSlot),
            (TransferOutcome::BadToTask, RefusalDetail::BadToTask),
            (TransferOutcome::ToSlotFull, RefusalDetail::ToSlotFull),
            (TransferOutcome::ToSlotOob, RefusalDetail::ToSlotOob),
            (
                TransferOutcome::Untransferable,
                RefusalDetail::Untransferable,
            ),
        ] {
            only_authority(transfer(outcome), detail);
        }
    }

    #[test]
    fn resolve_causes_all_map_to_authority_today() {
        assert_eq!(resolve(ResolveOutcome::Installed).status, Status::Ok);
        for (cause, detail) in [
            (ResolveOutcome::NoGrant, RefusalDetail::NoGrant),
            (ResolveOutcome::BadNameLen, RefusalDetail::BadNameLen),
            (ResolveOutcome::Missing, RefusalDetail::Missing),
            (ResolveOutcome::BadSlot, RefusalDetail::BadSlot),
        ] {
            only_authority(resolve(cause), detail);
        }
    }

    #[test]
    fn detail_codes_are_the_documented_abi() {
        // ADR-0061's table, as executable rows. A renumbering fails here
        // before it fails an agent in the field.
        for (d, code) in [
            (RefusalDetail::BadCap, 1),
            (RefusalDetail::UnknownDest, 2),
            (RefusalDetail::BadFromSlot, 3),
            (RefusalDetail::BadToTask, 4),
            (RefusalDetail::ToSlotFull, 5),
            (RefusalDetail::ToSlotOob, 6),
            (RefusalDetail::Untransferable, 7),
            (RefusalDetail::NoGrant, 8),
            (RefusalDetail::BadNameLen, 9),
            (RefusalDetail::Missing, 10),
            (RefusalDetail::BadSlot, 11),
            (RefusalDetail::NotIrqCap, 12),
        ] {
            assert_eq!(d.as_u64(), code);
        }
    }

    #[test]
    fn unpack_name_bounds_and_bytes() {
        assert_eq!(unpack_name(0, 0), None);
        assert_eq!(unpack_name(9, 0), None);
        let name = unpack_name(2, 0x6261).unwrap();
        assert_eq!(&name[..2], b"ab");
        assert_eq!(&name[2..], &[0u8; 6]);
        let full = unpack_name(8, u64::from_le_bytes(*b"deadbeef")).unwrap();
        assert_eq!(&full, b"deadbeef");
    }
}
