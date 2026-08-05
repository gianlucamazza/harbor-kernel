//! Minimal `SVC` imm decode (M5-P2) — pure, host-tested.

/// `svc #0` — ping / presence.
pub const SYS_PING: u16 = 0;

/// `svc #1` — cooperative exit from an EL0 multi-SVC session (no resume).
pub const SYS_EXIT: u16 = 1;

/// Result of decoding a user `SVC` immediate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syscall {
    /// Known no-op presence call (session may resume).
    Ping,
    /// End the EL0 session cleanly.
    Exit,
    /// Not in the table — refuse, do not invent behaviour.
    Unknown { imm: u16 },
}

/// Map architectural `SVC` imm → [`Syscall`].
#[inline]
pub const fn decode(imm: u16) -> Syscall {
    match imm {
        SYS_PING => Syscall::Ping,
        SYS_EXIT => Syscall::Exit,
        other => Syscall::Unknown { imm: other },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_and_exit() {
        assert_eq!(decode(0), Syscall::Ping);
        assert_eq!(decode(1), Syscall::Exit);
    }

    #[test]
    fn unknown_is_refused_not_aliased() {
        assert_eq!(decode(2), Syscall::Unknown { imm: 2 });
        assert_eq!(decode(0xffff), Syscall::Unknown { imm: 0xffff });
    }
}
