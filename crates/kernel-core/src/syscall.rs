//! `SVC` imm decode and the EL0 syscall ABI (ADR-0017) — pure, host-tested.
//!
//! # The register ABI
//!
//! An agent names a capability by **slot index into its own table**, never by
//! `CapId`: there is nothing outside that array for it to name (ADR-0017 §2).
//!
//! | call        | `x0` in | `x1` in | `x2` in | `x3` in | `x0` out   | `x1..x3` out  |
//! | ----------- | ------- | ------- | ------- | ------- | ---------- | ------------- |
//! | `SYS_PING`  | —       | —       | —       | —       | unchanged  | unchanged     |
//! | `SYS_EXIT`  | —       | —       | —       | —       | —          | —             |
//! | `SYS_PUTC`  | slot    | byte    | —       | —       | [`Status`] | unchanged     |
//! | `SYS_SEND`  | slot    | tag     | a       | b       | [`Status`] | unchanged     |
//! | `SYS_RECV`  | slot    | —       | —       | —       | [`Status`] | tag, a, b     |
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

/// `svc #2` — write the low 8 bits of `x1` to the console named by slot `x0`.
///
/// Requires a console capability, and is **denied by default**: an agent that
/// was not granted one is refused and the refusal is counted as an authority
/// violation (ADR-0017 §3).
///
/// **Transitional** (ADR-0017 §4). M8 replaces it with `SYS_SEND` on a console
/// endpoint: the agent-side ABI does not change — the slot already names the
/// right capability — only who drains the message.
pub const SYS_PUTC: u16 = 2;

/// `svc #3` — send a message through the capability in slot `x0`.
pub const SYS_SEND: u16 = 3;

/// `svc #4` — take a message from the capability in slot `x0` if one is queued.
///
/// Does **not** block. A blocking recv has to yield out of a live session, and
/// nothing performs a switch inside one yet — see ADR-0017's consequences.
pub const SYS_RECV: u16 = 4;

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
    /// Nothing queued. Only `SYS_RECV` produces this.
    Empty = 3,
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
    /// Emit one byte from saved `x0` via kernel TX; session may resume.
    Putc,
    /// Send through the capability named by the slot in `x0`.
    Send,
    /// Take a queued message from the capability named by the slot in `x0`.
    Recv,
    /// Not in the table — refuse, do not invent behaviour.
    Unknown { imm: u16 },
}

/// Map architectural `SVC` imm → [`Syscall`].
#[inline]
pub const fn decode(imm: u16) -> Syscall {
    match imm {
        SYS_PING => Syscall::Ping,
        SYS_EXIT => Syscall::Exit,
        SYS_PUTC => Syscall::Putc,
        SYS_SEND => Syscall::Send,
        SYS_RECV => Syscall::Recv,
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
        assert_eq!(decode(2), Syscall::Putc);
        assert_eq!(decode(3), Syscall::Send);
        assert_eq!(decode(4), Syscall::Recv);
    }

    #[test]
    fn unknown_is_refused_not_aliased() {
        // The first immediate past the table. It used to be 3; adding two calls
        // moved it, and an ABI that grows must not grow by accident — an
        // unimplemented call ends the session rather than aliasing a real one.
        assert_eq!(decode(5), Syscall::Unknown { imm: 5 });
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
    }
}
