//! Minimal `SVC` imm decode (M5-P2) — pure, host-tested.

/// `svc #0` — ping / presence.
pub const SYS_PING: u16 = 0;

/// Result of decoding a user `SVC` immediate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syscall {
    /// Known no-op presence call.
    Ping,
    /// Not in the v1 table — refuse, do not invent behaviour.
    Unknown { imm: u16 },
}

/// Map architectural `SVC` imm → [`Syscall`].
#[inline]
pub const fn decode(imm: u16) -> Syscall {
    match imm {
        SYS_PING => Syscall::Ping,
        other => Syscall::Unknown { imm: other },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_is_zero() {
        assert_eq!(decode(0), Syscall::Ping);
    }

    #[test]
    fn unknown_is_refused_not_aliased() {
        assert_eq!(decode(1), Syscall::Unknown { imm: 1 });
        assert_eq!(decode(0xffff), Syscall::Unknown { imm: 0xffff });
    }
}
