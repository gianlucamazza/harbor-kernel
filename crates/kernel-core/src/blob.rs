//! EL0 durable-blob endpoint protocol (P2).
//!
//! The first endpoint wire format deliberately fits one [`ipc::Message`]. Each
//! data field carries a four-bit length and up to seven bytes. Larger durable
//! keys or payloads remain valid for the EL1 backend, but are rejected at this
//! boundary rather than truncated.

use crate::ipc::Message;

pub const TAG_PUT: u32 = 0x1001;
pub const TAG_GET: u32 = 0x1002;
pub const TAG_OK: u32 = 0x1003;
pub const TAG_MISSING: u32 = 0x1004;
pub const TAG_BAD_REQUEST: u32 = 0x1005;
pub const TAG_STORE_ERROR: u32 = 0x1006;
pub const MAX_WIRE_BYTES: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub len: usize,
    pub bytes: [u8; MAX_WIRE_BYTES],
}

impl Field {
    pub const EMPTY: Self = Self {
        len: 0,
        bytes: [0; MAX_WIRE_BYTES],
    };

    pub const fn pack(bytes: &[u8]) -> u64 {
        let mut value = (bytes.len() as u64) << 56;
        let mut i = 0;
        while i < bytes.len() && i < MAX_WIRE_BYTES {
            value |= (bytes[i] as u64) << (i * 8);
            i += 1;
        }
        value
    }

    pub fn unpack(value: u64) -> Option<Self> {
        let len = (value >> 56) as usize;
        if len == 0 || len > MAX_WIRE_BYTES {
            return None;
        }
        let mut bytes = [0; MAX_WIRE_BYTES];
        let mut i = 0;
        while i < len {
            bytes[i] = (value >> (i * 8)) as u8;
            i += 1;
        }
        Some(Self { len, bytes })
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Put { key: Field, payload: Field },
    Get { key: Field },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    UnknownTag,
    BadKey,
    BadPayload,
}

pub fn decode(message: Message) -> Result<Request, DecodeError> {
    match message.tag {
        TAG_PUT => Ok(Request::Put {
            key: Field::unpack(message.a).ok_or(DecodeError::BadKey)?,
            payload: Field::unpack(message.b).ok_or(DecodeError::BadPayload)?,
        }),
        TAG_GET => Ok(Request::Get {
            key: Field::unpack(message.a).ok_or(DecodeError::BadKey)?,
        }),
        _ => Err(DecodeError::UnknownTag),
    }
}

pub const fn ok(payload: u64) -> Message {
    Message {
        tag: TAG_OK,
        a: payload,
        b: 0,
    }
}

pub const fn missing() -> Message {
    Message {
        tag: TAG_MISSING,
        a: 0,
        b: 0,
    }
}

pub const fn bad_request() -> Message {
    Message {
        tag: TAG_BAD_REQUEST,
        a: 0,
        b: 0,
    }
}

pub const fn store_error(code: u64) -> Message {
    Message {
        tag: TAG_STORE_ERROR,
        a: code,
        b: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get_wire_fields_round_trip() {
        let put = Message {
            tag: TAG_PUT,
            a: Field::pack(b"cfg"),
            b: Field::pack(b"persist"),
        };
        assert_eq!(
            decode(put),
            Ok(Request::Put {
                key: Field {
                    len: 3,
                    bytes: [b'c', b'f', b'g', 0, 0, 0, 0]
                },
                payload: Field {
                    len: 7,
                    bytes: *b"persist"
                },
            })
        );
        assert_eq!(
            decode(Message {
                tag: TAG_GET,
                a: Field::pack(b"cfg"),
                b: 0
            }),
            Ok(Request::Get {
                key: Field {
                    len: 3,
                    bytes: [b'c', b'f', b'g', 0, 0, 0, 0]
                },
            })
        );
    }

    #[test]
    fn malformed_and_unknown_messages_are_refused() {
        assert_eq!(
            decode(Message {
                tag: TAG_PUT,
                a: 0,
                b: Field::pack(b"x")
            }),
            Err(DecodeError::BadKey)
        );
        assert_eq!(
            decode(Message {
                tag: TAG_GET,
                a: Field::pack(b"x"),
                b: 1
            }),
            Ok(Request::Get {
                key: Field {
                    len: 1,
                    bytes: [b'x', 0, 0, 0, 0, 0, 0]
                },
            })
        );
        assert_eq!(
            decode(Message { tag: 0, a: 0, b: 0 }),
            Err(DecodeError::UnknownTag)
        );
    }
}
