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
/// On every refusal it is `None`: the kernel does not clear an agent's own
/// registers to make a point — `x1..x3` are meaningful only when `x0` says
/// `Ok` (ADR-0022 §4 wording, preserved byte-for-byte by the tests below).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: Status,
    pub payload: Option<[u64; 3]>,
    pub delta: StatDelta,
}

impl Reply {
    /// An authority refusal: `Authority`, no payload, exactly one count.
    const fn refused() -> Self {
        Self {
            status: Status::Authority,
            payload: None,
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
            delta: StatDelta {
                recvs: 1,
                ..StatDelta::default()
            },
        },
        RecvOutcome::Empty => Reply {
            status: Status::Empty,
            payload: None,
            delta: StatDelta {
                recv_empties: 1,
                ..StatDelta::default()
            },
        },
        // Someone else already waits there. The agent holds what it named, so
        // this never touches the authority counter (ADR-0022 §4).
        RecvOutcome::Busy => Reply::bare(Status::Busy),
        RecvOutcome::Cancelled => Reply::bare(Status::Cancelled),
        RecvOutcome::BadCap => Reply::refused(),
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
            delta: StatDelta {
                sends: 1,
                ..StatDelta::default()
            },
        },
        SendOutcome::Full => Reply::bare(Status::Full),
        SendOutcome::Refused => Reply::refused(),
    }
}

/// What the wait-on-IRQ path answered (ADR-0028/0030).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitIrqOutcome {
    Woken,
    /// Cookie or task already armed — state, not authority (ADR-0028).
    Busy,
    /// Slot, hold, or cookie lookup failed.
    NoAuthority,
}

pub fn wait_irq(outcome: WaitIrqOutcome) -> Reply {
    match outcome {
        WaitIrqOutcome::Woken => Reply {
            status: Status::Ok,
            payload: None,
            delta: StatDelta {
                wait_irqs: 1,
                ..StatDelta::default()
            },
        },
        WaitIrqOutcome::Busy => Reply::bare(Status::Busy),
        WaitIrqOutcome::NoAuthority => Reply::refused(),
    }
}

/// What the transfer path answered (ADR-0041/0054/0055).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferOutcome {
    Moved,
    /// `x2` named a dest this kernel does not decode.
    BadDest,
    /// Any refusal from `transfer_held*` — slot, target, band, staleness.
    Refused,
}

pub fn transfer(outcome: TransferOutcome) -> Reply {
    match outcome {
        TransferOutcome::Moved => Reply::bare(Status::Ok),
        TransferOutcome::BadDest | TransferOutcome::Refused => Reply::refused(),
    }
}

/// What the resolve path answered (ADR-0039/0052).
///
/// Four distinct causes, one mapping today — kept as variants so a refusal
/// taxonomy (planned successor) is a mapping change here, not a reshape of
/// the callers.
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
        ResolveOutcome::NoGrant
        | ResolveOutcome::BadNameLen
        | ResolveOutcome::Missing
        | ResolveOutcome::BadSlot => Reply::refused(),
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

    fn only_authority(r: Reply) {
        assert_eq!(r.status, Status::Authority);
        assert_eq!(r.payload, None);
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
        // The reply registers are meaningful only when x0 says Ok; every
        // non-Ok outcome must leave the agent's x1..x3 alone.
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
        only_authority(recv(RecvOutcome::BadCap));
    }

    #[test]
    fn send_maps_full_off_the_authority_counter() {
        assert_eq!(send(SendOutcome::Sent).status, Status::Ok);
        assert_eq!(send(SendOutcome::Sent).delta.sends, 1);
        let full = send(SendOutcome::Full);
        assert_eq!(full.status, Status::Full);
        assert_eq!(full.delta, StatDelta::default());
        only_authority(send(SendOutcome::Refused));
    }

    #[test]
    fn wait_irq_busy_is_state_not_authority() {
        assert_eq!(wait_irq(WaitIrqOutcome::Woken).status, Status::Ok);
        assert_eq!(wait_irq(WaitIrqOutcome::Woken).delta.wait_irqs, 1);
        let busy = wait_irq(WaitIrqOutcome::Busy);
        assert_eq!(busy.status, Status::Busy);
        assert_eq!(busy.delta, StatDelta::default());
        only_authority(wait_irq(WaitIrqOutcome::NoAuthority));
    }

    #[test]
    fn transfer_ok_counts_nothing_and_every_refusal_counts_once() {
        let ok = transfer(TransferOutcome::Moved);
        assert_eq!(ok.status, Status::Ok);
        assert_eq!(ok.delta, StatDelta::default());
        only_authority(transfer(TransferOutcome::BadDest));
        only_authority(transfer(TransferOutcome::Refused));
    }

    #[test]
    fn resolve_causes_all_map_to_authority_today() {
        assert_eq!(resolve(ResolveOutcome::Installed).status, Status::Ok);
        for cause in [
            ResolveOutcome::NoGrant,
            ResolveOutcome::BadNameLen,
            ResolveOutcome::Missing,
            ResolveOutcome::BadSlot,
        ] {
            only_authority(resolve(cause));
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
