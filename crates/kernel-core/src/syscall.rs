//! `SVC` imm decode and the EL0 syscall ABI (ADR-0017) — pure, host-tested.
//!
//! # The register ABI
//!
//! An agent names a capability by **slot index into its own table**, never by
//! `CapId`: there is nothing outside that array for it to name (ADR-0017 §2).
//!
//! | call           | `x0` in | `x1` in | `x2` in | `x3` in | `x0` out   | `x1..x3` out  |
//! | -------------- | ------- | ------- | ------- | ------- | ---------- | ------------- |
//! | `SYS_PING`     | —       | —       | —       | —       | unchanged  | unchanged     |
//! | `SYS_EXIT`     | —       | —       | —       | —       | —          | —             |
//! | `SYS_SEND`     | slot    | tag     | a       | b       | [`Status`] | unchanged     |
//! | `SYS_RECV`     | slot    | —       | —       | —       | [`Status`] | tag, a, b     |
//! | `SYS_TRY_RECV` | slot    | —       | —       | —       | [`Status`] | tag, a, b     |
//! | `SYS_WAIT_IRQ` | slot    | —       | —       | —       | [`Status`] | unchanged     |
//! | `SYS_RESOLVE`  | slot    | name_len| name_le | —       | [`Status`] | unchanged     |
//!
//! Imm 2 is **unused** (formerly transitional `SYS_PUTC`, removed in M8). It
//! decodes as [`Syscall::Unknown`] so a stale agent image that still issues
//! `svc #2` is refused rather than aliased onto `SYS_SEND`.
//!
//! The kernel writes `x0..x3` and nothing else: those four are the reply, and
//! any other register is the agent's own context, which answering a syscall has
//! no business changing.
//!
//! `x1..x3` carry a payload **only when `x0` is [`Status::Ok`]**. On a refusal
//! the kernel writes the status and stops, so those registers keep whatever the
//! agent had in them — it is not handed stale kernel data, and it is not
//! handed zeroes either. An agent that reads them without checking `x0` is
//! reading its own scratch.

/// `svc #0` — ping / presence.
pub const SYS_PING: u16 = 0;

/// `svc #1` — cooperative exit from an EL0 multi-SVC session (no resume).
pub const SYS_EXIT: u16 = 1;

// Imm 2 was `SYS_PUTC` (transitional, ADR-0017 §4). M8 removed it: console
// output is `SYS_SEND` on a console endpoint. The number is left unassigned so
// a binary still using `svc #2` hits [`Syscall::Unknown`] rather than SEND.

/// `svc #3` — send a message through the capability in slot `x0`.
///
/// Console output uses this with [`crate::prog::CONSOLE_TAG_BYTE`] and the byte
/// in `a` (M8).
pub const SYS_SEND: u16 = 3;

/// `svc #4` — take a message from the capability in slot `x0`, **waiting** if
/// none is queued.
///
/// The agent's text is unaware of the wait: it executes the `svc` and, whenever
/// it next runs, finds [`Status::Ok`] with the message in `x1..x3`. Waiting is
/// the kernel's, not the program's (ADR-0022 §1). Never returns
/// [`Status::Empty`] — an agent that must not wait calls [`SYS_TRY_RECV`].
pub const SYS_RECV: u16 = 4;

/// `svc #5` — take a message from the capability in slot `x0` if one is queued,
/// and answer [`Status::Empty`] if none is.
///
/// The non-blocking half of the pair, kept as its own immediate rather than as a
/// flag: an agent that must not park — a poll loop, an interrupt service body —
/// needs to say so at the call, and a blocking-only recv would take that away
/// (ADR-0022 §4).
pub const SYS_TRY_RECV: u16 = 5;

/// `svc #6` — wait until the IRQ cookie named by the notification in slot `x0`
/// signals (ADR-0030 / K1 remainder).
///
/// Authority is the IRQ-notification cap, not a raw cookie. No payload: `x0`
/// carries [`Status`] only.
pub const SYS_WAIT_IRQ: u16 = 6;

/// `svc #7` — resolve a short name into an empty local slot (ADR-0039 / P5).
///
/// `x0` = empty slot; `x1` = name length (1..=8); `x2` = name bytes LE-packed.
/// Success installs a `CapId` into the slot; the agent never sees the raw id.
pub const SYS_RESOLVE: u16 = 7;

/// What an agent reads in `x0` after a call that names a capability.
///
/// A number rather than a `Result`, because a number is what an `eret` can
/// carry. The three failures are kept apart for the reason the kernel's own
/// refusal counters are: a full mailbox is flow control, and an unheld slot is
/// an attempt to exceed the authority the agent was granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Status {
    Ok = 0,
    /// Slot out of range, empty, or naming a capability the task does not hold.
    Authority = 1,
    /// The mailbox is full. Flow control, not a violation.
    Full = 2,
    /// Nothing queued. Only `SYS_TRY_RECV` produces this — `SYS_RECV` waits.
    Empty = 3,
    /// Someone else is already waiting on this endpoint.
    ///
    /// One endpoint, one waiter (ADR-0017's topology, which ADR-0022 does not
    /// widen). Kept apart from [`Self::Authority`] because it is not one: the
    /// agent holds the capability it named, and the kernel counts this as a
    /// *state* refusal. Folding it into `Authority` would inflate the number the
    /// boot check asserts exactly.
    Busy = 4,
    /// The wait was cancelled by a supervisor (`sched::cancel_blocked`, ADR-0025).
    /// Not an authority failure: the agent held the cap; the park was aborted.
    Cancelled = 5,
}

impl Status {
    /// The value the agent finds in `x0`.
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self as u64
    }
}

/// Result of decoding a user `SVC` immediate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syscall {
    /// Known no-op presence call (session may resume).
    Ping,
    /// End the EL0 session cleanly.
    Exit,
    /// Send through the capability named by the slot in `x0`.
    Send,
    /// Take a message from the capability named by the slot in `x0`, waiting
    /// for one if the mailbox is empty.
    Recv,
    /// Take a queued message from the slot in `x0`, or answer `Empty`.
    TryRecv,
    /// Wait on the IRQ notification in slot `x0` (ADR-0030).
    WaitIrq,
    /// Resolve a short name into empty slot `x0` (ADR-0039).
    Resolve,
    /// Not in the table — refuse, do not invent behaviour.
    Unknown { imm: u16 },
}

/// Map architectural `SVC` imm → [`Syscall`].
#[inline]
pub const fn decode(imm: u16) -> Syscall {
    match imm {
        SYS_PING => Syscall::Ping,
        SYS_EXIT => Syscall::Exit,
        SYS_SEND => Syscall::Send,
        SYS_RECV => Syscall::Recv,
        SYS_TRY_RECV => Syscall::TryRecv,
        SYS_WAIT_IRQ => Syscall::WaitIrq,
        SYS_RESOLVE => Syscall::Resolve,
        other => Syscall::Unknown { imm: other },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_imms() {
        assert_eq!(decode(0), Syscall::Ping);
        assert_eq!(decode(1), Syscall::Exit);
        assert_eq!(decode(3), Syscall::Send);
        assert_eq!(decode(4), Syscall::Recv);
        assert_eq!(decode(5), Syscall::TryRecv);
        assert_eq!(decode(6), Syscall::WaitIrq);
        assert_eq!(decode(7), Syscall::Resolve);
    }

    #[test]
    fn former_putc_imm_is_unknown() {
        // M8: imm 2 is unassigned. A stale `svc #2` must not alias SEND.
        assert_eq!(decode(2), Syscall::Unknown { imm: 2 });
    }

    #[test]
    fn the_two_recvs_are_different_calls() {
        // Not a tautology: the pair exists so an agent can *say* whether it may
        // wait, and that only works while the two immediates decode apart. A
        // blocking-only ABI would be one that answered `Recv` to both.
        assert_ne!(decode(SYS_RECV), decode(SYS_TRY_RECV));
    }

    #[test]
    fn unknown_is_refused_not_aliased() {
        // First unused imm after the last known syscall (RESOLVE = 7).
        assert_eq!(decode(8), Syscall::Unknown { imm: 8 });
        assert_eq!(decode(0xffff), Syscall::Unknown { imm: 0xffff });
    }

    #[test]
    fn status_values_are_the_abi_and_are_distinct() {
        // These numbers are read by user code that this kernel does not
        // compile. Renumbering them is an ABI break, not a refactor.
        assert_eq!(Status::Ok.as_u64(), 0);
        assert_eq!(Status::Authority.as_u64(), 1);
        assert_eq!(Status::Full.as_u64(), 2);
        assert_eq!(Status::Empty.as_u64(), 3);
        assert_eq!(Status::Busy.as_u64(), 4);
        assert_eq!(Status::Cancelled.as_u64(), 5);
    }
}
