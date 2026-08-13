//! EL1 network-service wire protocol (ADR-0104).
//!
//! Messages carry packet-pool identity only: slot, generation, and length.
//! Physical addresses, descriptor indices, and MMIO never cross this boundary.

use crate::ipc::Message;
use crate::virtio::{PACKET_BYTES, PacketToken};

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

#[cfg(test)]
mod tests {
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
