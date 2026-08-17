//! EL1 network-service wire protocol (ADR-0104).
//!
//! Messages carry packet-pool identity only: slot, generation, and length.
//! Physical addresses, descriptor indices, and MMIO never cross this boundary.

use crate::ipc::Message;

pub const TAG_TX_SUBMIT: u32 = 0x1101;
pub const TAG_RX_RETURN: u32 = 0x1102;
pub const TAG_TX_COMPLETE: u32 = 0x1103;
pub const TAG_RX_AVAILABLE: u32 = 0x1104;
pub const TAG_REFUSED: u32 = 0x1105;

const SLOT_BITS: u64 = 8;
const GENERATION_BITS: u64 = 32;
const LEN_SHIFT: u32 = (SLOT_BITS + GENERATION_BITS) as u32;
const LEN_MASK: u64 = 0xFFFF;
const RESERVED_MASK: u64 = !((1 << (LEN_SHIFT + 16)) - 1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    TxSubmit(PacketToken),
    RxReturn(PacketToken),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    UnknownTag,
    ReservedBits,
    LengthOutOfRange,
}

pub const fn tx_submit(token: PacketToken) -> Message {
    Message {
        tag: TAG_TX_SUBMIT,
        a: pack(token),
        b: 0,
    }
}

pub const fn rx_return(token: PacketToken) -> Message {
    Message {
        tag: TAG_RX_RETURN,
        a: pack(token),
        b: 0,
    }
}

/// Packed token representation used in the IPC message ABI.
pub const fn packed_token(token: PacketToken) -> u64 {
    pack(token)
}

pub fn decode(message: Message) -> Result<Request, DecodeError> {
    let token = unpack(message.a)?;
    match message.tag {
        TAG_TX_SUBMIT => Ok(Request::TxSubmit(token)),
        TAG_RX_RETURN => Ok(Request::RxReturn(token)),
        _ => Err(DecodeError::UnknownTag),
    }
}

pub const fn tx_complete(token: PacketToken) -> Message {
    Message {
        tag: TAG_TX_COMPLETE,
        a: pack(token),
        b: 0,
    }
}

pub const fn rx_available(token: PacketToken) -> Message {
    Message {
        tag: TAG_RX_AVAILABLE,
        a: pack(token),
        b: 0,
    }
}

pub const fn refused(code: u64) -> Message {
    Message {
        tag: TAG_REFUSED,
        a: code,
        b: 0,
    }
}

const fn pack(token: PacketToken) -> u64 {
    token.slot as u64 | (token.generation as u64) << SLOT_BITS | (token.len as u64) << LEN_SHIFT
}

fn unpack(value: u64) -> Result<PacketToken, DecodeError> {
    if value & RESERVED_MASK != 0 {
        return Err(DecodeError::ReservedBits);
    }
    let len = ((value >> LEN_SHIFT) & LEN_MASK) as usize;
    if len > PACKET_BYTES {
        return Err(DecodeError::LengthOutOfRange);
    }
    Ok(PacketToken {
        slot: value as u8,
        generation: (value >> SLOT_BITS) as u32,
        len: len as u16,
    })
}

/// Fixed packet slot size mandated by ADR-0104.
pub const PACKET_BYTES: usize = 2 * 1024;
/// Bounded first-slice pool size.
pub const PACKET_SLOTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    TxAgent,
    TxService,
    RxService,
    RxAgent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    generation: u32,
    state: SlotState,
}

/// A bounded TX/RX packet ownership table.
///
/// The first half is TX-owned by the agent and the second half is RX-owned by
/// the service.  Tokens contain only slot, generation, and length; no token
/// exposes a physical address or a kernel pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketPool {
    slots: [Slot; PACKET_SLOTS],
}

/// A packet reference carried across the service boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketToken {
    pub slot: u8,
    pub generation: u32,
    pub len: u16,
}

/// Why a packet operation was refused before touching device state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketError {
    SlotOutOfRange,
    StaleGeneration,
    WrongDirection,
    WrongOwner,
    Oversize,
}

impl PacketPool {
    /// Create a reset pool.  TX slots start with the agent; RX slots with EL1.
    pub const fn new() -> Self {
        let mut slots = [Slot {
            generation: 0,
            state: SlotState::TxAgent,
        }; PACKET_SLOTS];
        let mut i = PACKET_SLOTS / 2;
        while i < PACKET_SLOTS {
            slots[i].state = SlotState::RxService;
            i += 1;
        }
        Self { slots }
    }

    /// Submit a frame from an agent TX slot to the EL1 service.
    pub fn submit_tx(
        &mut self,
        slot: usize,
        generation: u32,
        len: usize,
    ) -> Result<PacketToken, PacketError> {
        let current = self.checked_slot(slot, generation)?;
        if slot >= PACKET_SLOTS / 2 {
            return Err(PacketError::WrongDirection);
        }
        if current.state != SlotState::TxAgent {
            return Err(PacketError::WrongOwner);
        }
        let len = u16::try_from(len).map_err(|_| PacketError::Oversize)?;
        if usize::from(len) > PACKET_BYTES {
            return Err(PacketError::Oversize);
        }
        self.slots[slot].state = SlotState::TxService;
        Ok(PacketToken {
            slot: slot as u8,
            generation,
            len,
        })
    }

    /// Accept a token received over the service endpoint after validating its
    /// slot, generation, length, direction, and current owner.
    pub fn accept_tx(&mut self, token: PacketToken) -> Result<(), PacketError> {
        let slot = self.checked_token(token)?;
        if usize::from(token.slot) >= PACKET_SLOTS / 2 {
            return Err(PacketError::WrongDirection);
        }
        if slot.state != SlotState::TxAgent {
            return Err(PacketError::WrongOwner);
        }
        self.slots[usize::from(token.slot)].state = SlotState::TxService;
        Ok(())
    }

    /// Return a completed TX slot to the agent.
    pub fn complete_tx(&mut self, token: PacketToken) -> Result<(), PacketError> {
        let slot = self.checked_token(token)?;
        if slot.state != SlotState::TxService {
            return Err(PacketError::WrongOwner);
        }
        self.slots[usize::from(token.slot)].state = SlotState::TxAgent;
        Ok(())
    }

    /// Publish an RX frame into an EL1-owned RX slot and notify the agent.
    pub fn publish_rx(&mut self, slot: usize, len: usize) -> Result<PacketToken, PacketError> {
        if slot >= PACKET_SLOTS {
            return Err(PacketError::SlotOutOfRange);
        }
        if slot < PACKET_SLOTS / 2 {
            return Err(PacketError::WrongDirection);
        }
        let current = self.slots[slot];
        if current.state != SlotState::RxService {
            return Err(PacketError::WrongOwner);
        }
        let len = u16::try_from(len).map_err(|_| PacketError::Oversize)?;
        if usize::from(len) > PACKET_BYTES {
            return Err(PacketError::Oversize);
        }
        self.slots[slot].state = SlotState::RxAgent;
        Ok(PacketToken {
            slot: slot as u8,
            generation: current.generation,
            len,
        })
    }

    /// Return an RX slot after the agent consumed it.
    pub fn return_rx(&mut self, token: PacketToken) -> Result<(), PacketError> {
        let slot = self.checked_token(token)?;
        if slot.state != SlotState::RxAgent {
            return Err(PacketError::WrongOwner);
        }
        self.slots[usize::from(token.slot)].state = SlotState::RxService;
        Ok(())
    }

    /// Reset the device contract and invalidate every outstanding token.
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.generation = slot.generation.wrapping_add(1);
            slot.state = SlotState::TxAgent;
        }
        let mut i = PACKET_SLOTS / 2;
        while i < PACKET_SLOTS {
            self.slots[i].state = SlotState::RxService;
            i += 1;
        }
    }

    fn checked_slot(&self, slot: usize, generation: u32) -> Result<Slot, PacketError> {
        let current = *self.slots.get(slot).ok_or(PacketError::SlotOutOfRange)?;
        if current.generation != generation {
            return Err(PacketError::StaleGeneration);
        }
        Ok(current)
    }

    fn checked_token(&self, token: PacketToken) -> Result<Slot, PacketError> {
        if usize::from(token.len) > PACKET_BYTES {
            return Err(PacketError::Oversize);
        }
        self.checked_slot(usize::from(token.slot), token.generation)
    }
}

impl Default for PacketPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tx_and_rx_ownership_is_directional() {
        let mut pool = PacketPool::new();
        let tx = pool.submit_tx(0, 0, 64).unwrap();
        assert_eq!(pool.submit_tx(0, 0, 64), Err(PacketError::WrongOwner));
        assert_eq!(pool.publish_rx(0, 64), Err(PacketError::WrongDirection));
        pool.complete_tx(tx).unwrap();
        let rx = pool.publish_rx(PACKET_SLOTS / 2, 128).unwrap();
        assert_eq!(pool.return_rx(rx), Ok(()));
        assert_eq!(pool.return_rx(rx), Err(PacketError::WrongOwner));
    }

    #[test]
    fn malformed_lengths_are_refused() {
        let mut pool = PacketPool::new();
        assert_eq!(
            pool.submit_tx(0, 0, PACKET_BYTES + 1),
            Err(PacketError::Oversize)
        );
        assert_eq!(
            pool.publish_rx(PACKET_SLOTS / 2, PACKET_BYTES + 1),
            Err(PacketError::Oversize)
        );
    }

    #[test]
    fn reset_invalidates_outstanding_tokens() {
        let mut pool = PacketPool::new();
        let tx = pool.submit_tx(0, 0, 1).unwrap();
        let rx = pool.publish_rx(PACKET_SLOTS / 2, 1).unwrap();
        pool.reset();
        assert_eq!(pool.complete_tx(tx), Err(PacketError::StaleGeneration));
        assert_eq!(pool.return_rx(rx), Err(PacketError::StaleGeneration));
        let fresh = pool.submit_tx(0, 1, 1).unwrap();
        assert_eq!(fresh.generation, 1);
    }

    use super::*;

    const TOKEN: PacketToken = PacketToken {
        slot: 3,
        generation: 0xAABB_CCDD,
        len: 1500,
    };

    #[test]
    fn tx_and_return_round_trip_identity_only() {
        assert_eq!(decode(tx_submit(TOKEN)), Ok(Request::TxSubmit(TOKEN)));
        assert_eq!(decode(rx_return(TOKEN)), Ok(Request::RxReturn(TOKEN)));
        assert_eq!(tx_complete(TOKEN).a, rx_available(TOKEN).a);
    }

    #[test]
    fn unknown_tag_is_refused() {
        assert_eq!(
            decode(Message { tag: 0, a: 0, b: 0 }),
            Err(DecodeError::UnknownTag)
        );
    }

    #[test]
    fn reserved_bits_are_refused() {
        let message = Message {
            tag: TAG_TX_SUBMIT,
            a: pack(TOKEN) | (1 << 56),
            b: 0,
        };
        assert_eq!(decode(message), Err(DecodeError::ReservedBits));
    }

    #[test]
    fn oversized_length_is_refused() {
        let message = Message {
            tag: TAG_TX_SUBMIT,
            a: pack(PacketToken {
                len: (PACKET_BYTES + 1) as u16,
                ..TOKEN
            }),
            b: 0,
        };
        assert_eq!(decode(message), Err(DecodeError::LengthOutOfRange));
    }

    #[test]
    fn response_helpers_preserve_token_fields() {
        assert_eq!(
            decode(Message {
                tag: TAG_TX_COMPLETE,
                a: pack(TOKEN),
                b: 0
            }),
            Err(DecodeError::UnknownTag)
        );
        assert_eq!(tx_complete(TOKEN).tag, TAG_TX_COMPLETE);
        assert_eq!(rx_available(TOKEN).tag, TAG_RX_AVAILABLE);
        assert_eq!(
            refused(7),
            Message {
                tag: TAG_REFUSED,
                a: 7,
                b: 0
            }
        );
    }
}
