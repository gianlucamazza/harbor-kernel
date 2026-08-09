//! SD card initialisation state machine (pure, host-testable).
//!
//! Policy only: which command to issue in each state and what a response
//! means. The driver executes — it owns MMIO, interrupt polling and the
//! response registers, and feeds `RESP0` back here. States are data, so the
//! whole init walk (and every refusal) is a host test.
//!
//! Scope is ADR-0066's: **SDHC/SDXC only** (CMD8-answering, ACMD41 with
//! HCS, CCS=1 accepted; CCS=0 refused as `Unsupported`), 1-bit bus, no
//! switch to high-speed.

use crate::sdhci::{DataDir, RespType};

/// CMD8 check pattern: 2.7–3.6 V (0x1) + pattern 0xAA, echoed by the card.
pub const IF_COND_ARG: u32 = 0x1AA;

/// ACMD41: HCS (bit 30) + 3.2–3.4 V window. The card answers in OCR.
pub const OP_COND_ARG: u32 = (1 << 30) | 0x0030_0000;

/// OCR bit 31: power-up done (0 = still busy, retry ACMD41).
pub const OCR_BUSY_DONE: u32 = 1 << 31;
/// OCR bit 30: card capacity status — set = SDHC/SDXC (block addressing).
pub const OCR_CCS: u32 = 1 << 30;

/// ACMD41 retry bound. Cards must finish power-up within 1 s; at one poll
/// per millisecond-class loop this is generous without being unbounded.
pub const OP_COND_ATTEMPTS: u32 = 1000;

/// One command the driver must issue, as this machine wants it encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CmdReq {
    pub index: u8,
    pub arg: u32,
    pub resp: RespType,
    pub dir: DataDir,
    /// R3 has no CRC and a reserved index field — the driver must strip
    /// both checks or good cards fail.
    pub checks: bool,
}

/// Where the init walk stands. `advance` consumes the response to the
/// command [`request`] asked for in this state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitState {
    /// CMD0 — no response.
    GoIdle,
    /// CMD8 — voltage/pattern echo; a timeout here is a legacy card.
    IfCond,
    /// CMD55 (rca 0) prefixing the next ACMD41.
    OpCondPrefix { attempt: u32 },
    /// ACMD41 — loop until OCR busy clears.
    OpCond { attempt: u32 },
    /// CMD2 — CID (long response, content unused here).
    Cid,
    /// CMD3 — the card publishes its RCA.
    Rca,
    /// CMD7 — select the card (into `Transfer` state).
    Select { rca: u16 },
    /// Card ready for CMD17/CMD24.
    Done { rca: u16, high_capacity: bool },
}

/// Why the walk refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError {
    /// Legacy or SDSC card — outside ADR-0066's slice.
    Unsupported,
    /// CMD8 went unanswered: an empty slot or a pre-v2 card — this level
    /// cannot tell the two apart, and does not pretend to.
    NoResponse,
    /// CMD8 echo mismatch: the card answered with a different voltage or
    /// pattern than sent.
    BadIfCond,
    /// The card never finished power-up within [`OP_COND_ATTEMPTS`].
    PowerUpTimeout,
    /// A response arrived for a state that expects none.
    Protocol,
}

/// The command to issue in `state`; `None` when the walk is done.
pub const fn request(state: InitState) -> Option<CmdReq> {
    match state {
        InitState::GoIdle => Some(CmdReq {
            index: 0,
            arg: 0,
            resp: RespType::None,
            dir: DataDir::None,
            checks: false,
        }),
        InitState::IfCond => Some(CmdReq {
            index: 8,
            arg: IF_COND_ARG,
            resp: RespType::Short,
            dir: DataDir::None,
            checks: true,
        }),
        InitState::OpCondPrefix { .. } => Some(CmdReq {
            index: 55,
            arg: 0,
            resp: RespType::Short,
            dir: DataDir::None,
            checks: true,
        }),
        InitState::OpCond { .. } => Some(CmdReq {
            index: 41,
            arg: OP_COND_ARG,
            resp: RespType::Short,
            dir: DataDir::None,
            checks: false,
        }),
        InitState::Cid => Some(CmdReq {
            index: 2,
            arg: 0,
            resp: RespType::Long,
            dir: DataDir::None,
            checks: true,
        }),
        InitState::Rca => Some(CmdReq {
            index: 3,
            arg: 0,
            resp: RespType::Short,
            dir: DataDir::None,
            checks: true,
        }),
        InitState::Select { rca } => Some(CmdReq {
            index: 7,
            arg: (rca as u32) << 16,
            resp: RespType::ShortBusy,
            dir: DataDir::None,
            checks: true,
        }),
        InitState::Done { .. } => None,
    }
}

/// Consume the response (`RESP0`) to the state's command.
pub const fn advance(state: InitState, resp0: u32) -> Result<InitState, InitError> {
    match state {
        InitState::GoIdle => Ok(InitState::IfCond),
        InitState::IfCond => {
            if resp0 & 0xFFF == IF_COND_ARG {
                Ok(InitState::OpCondPrefix { attempt: 0 })
            } else {
                Err(InitError::BadIfCond)
            }
        }
        InitState::OpCondPrefix { attempt } => Ok(InitState::OpCond { attempt }),
        InitState::OpCond { attempt } => {
            if resp0 & OCR_BUSY_DONE == 0 {
                if attempt + 1 >= OP_COND_ATTEMPTS {
                    Err(InitError::PowerUpTimeout)
                } else {
                    Ok(InitState::OpCondPrefix {
                        attempt: attempt + 1,
                    })
                }
            } else if resp0 & OCR_CCS == 0 {
                // SDSC: byte addressing — the duality ADR-0066 refuses.
                Err(InitError::Unsupported)
            } else {
                Ok(InitState::Cid)
            }
        }
        InitState::Cid => Ok(InitState::Rca),
        InitState::Rca => Ok(InitState::Select {
            rca: (resp0 >> 16) as u16,
        }),
        InitState::Select { rca } => Ok(InitState::Done {
            rca,
            high_capacity: true,
        }),
        InitState::Done { .. } => Err(InitError::Protocol),
    }
}

/// Consume a command timeout in `state`. Only CMD8 interprets one (empty
/// slot or pre-v2 card); anywhere else it is a dead card/slot the driver
/// reports.
pub const fn on_timeout(state: InitState) -> InitError {
    match state {
        InitState::IfCond => InitError::NoResponse,
        _ => InitError::PowerUpTimeout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the machine with canned responses, returning the walk.
    fn walk(responses: &[(InitState, u32)]) -> Result<InitState, InitError> {
        let mut state = InitState::GoIdle;
        for &(expect, resp0) in responses {
            assert_eq!(state, expect, "walk diverged before {expect:?}");
            assert!(request(state).is_some(), "no command for {state:?}");
            state = advance(state, resp0)?;
        }
        Ok(state)
    }

    #[test]
    fn golden_sdhc_walk() {
        let done = walk(&[
            (InitState::GoIdle, 0),
            (InitState::IfCond, IF_COND_ARG),
            (InitState::OpCondPrefix { attempt: 0 }, 0x0000_0120),
            // First ACMD41: still busy.
            (InitState::OpCond { attempt: 0 }, 0x00FF_8000),
            (InitState::OpCondPrefix { attempt: 1 }, 0x0000_0120),
            // Second: done + CCS.
            (
                InitState::OpCond { attempt: 1 },
                OCR_BUSY_DONE | OCR_CCS | 0x00FF_8000,
            ),
            (InitState::Cid, 0),
            (InitState::Rca, 0xB368_0500),
            (InitState::Select { rca: 0xB368 }, 0x0000_0700),
        ])
        .unwrap();
        assert_eq!(
            done,
            InitState::Done {
                rca: 0xB368,
                high_capacity: true
            }
        );
        assert_eq!(request(done), None);
    }

    #[test]
    fn sdsc_card_is_refused() {
        // Power-up done but CCS clear: standard-capacity card.
        let r = advance(InitState::OpCond { attempt: 0 }, OCR_BUSY_DONE);
        assert_eq!(r, Err(InitError::Unsupported));
    }

    #[test]
    fn cmd8_silence_is_no_response_not_a_capacity_claim() {
        // Empty slot and pre-v2 card are indistinguishable here; the error
        // says exactly that, and nothing about the card that may not exist.
        assert_eq!(on_timeout(InitState::IfCond), InitError::NoResponse);
    }

    #[test]
    fn bad_if_cond_echo_is_refused() {
        assert_eq!(advance(InitState::IfCond, 0x155), Err(InitError::BadIfCond));
    }

    #[test]
    fn op_cond_bound_exhausts() {
        let mut state = InitState::OpCondPrefix { attempt: 0 };
        let mut steps = 0u32;
        loop {
            state = match advance(state, 0x00FF_8000) {
                Ok(s) => s,
                Err(e) => {
                    assert_eq!(e, InitError::PowerUpTimeout);
                    break;
                }
            };
            steps += 1;
            assert!(steps < 3 * OP_COND_ATTEMPTS, "bound never fired");
        }
    }

    #[test]
    fn rca_is_taken_from_the_response_high_half() {
        assert_eq!(
            advance(InitState::Rca, 0x1234_0500),
            Ok(InitState::Select { rca: 0x1234 })
        );
    }

    #[test]
    fn acmd41_request_strips_checks_and_cmd55_keeps_them() {
        let acmd = request(InitState::OpCond { attempt: 0 }).unwrap();
        assert!(!acmd.checks, "R3 has no CRC and a reserved index");
        let prefix = request(InitState::OpCondPrefix { attempt: 0 }).unwrap();
        assert!(prefix.checks);
    }
}
