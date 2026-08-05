//! Minimal `SVC` imm decode (M5-P2) — pure, host-tested.

/// `svc #0` — ping / presence.
pub const SYS_PING: u16 = 0;

/// `svc #1` — cooperative exit from an EL0 multi-SVC session (no resume).
pub const SYS_EXIT: u16 = 1;

/// `svc #2` — write low 8 bits of `x0` to the kernel console (TX).
pub const SYS_PUTC: u16 = 2;

/// Result of decoding a user `SVC` immediate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syscall {
    /// Known no-op presence call (session may resume).
    Ping,
    /// End the EL0 session cleanly.
    Exit,
    /// Emit one byte from saved `x0` via kernel TX; session may resume.
    Putc,
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
    }

    #[test]
    fn unknown_is_refused_not_aliased() {
        assert_eq!(decode(3), Syscall::Unknown { imm: 3 });
        assert_eq!(decode(0xffff), Syscall::Unknown { imm: 0xffff });
    }
}
