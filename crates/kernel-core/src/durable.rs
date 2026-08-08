//! Durable multi-blob wire format (ADR-0045 / P2) — pure, host-tested.
//!
//! Fixed-capacity encoding into a byte buffer (image section). Not a filesystem.

/// `b"DURB"` little-endian.
pub const MAGIC: u32 = u32::from_le_bytes(*b"DURB");
pub const VERSION: u32 = 1;
pub const MAX_BLOBS: usize = 4;
pub const MAX_KEY_LEN: usize = 16;
pub const MAX_PAYLOAD: usize = 64;
/// Header + 4 records worst case (key + len + payload padded).
pub const REGION_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    TooLarge,
    BadKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    BadMagic,
    BadVersion,
    BadCount,
    Truncated,
    BadKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blob<'a> {
    pub key: &'a [u8],
    pub payload: &'a [u8],
}

fn write_u32(buf: &mut [u8], off: usize, v: u32) -> Result<(), EncodeError> {
    let end = off.checked_add(4).ok_or(EncodeError::TooLarge)?;
    let s = buf.get_mut(off..end).ok_or(EncodeError::TooLarge)?;
    s.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn read_u32(buf: &[u8], off: usize) -> Result<u32, DecodeError> {
    let end = off.checked_add(4).ok_or(DecodeError::Truncated)?;
    let s = buf.get(off..end).ok_or(DecodeError::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Encode `blobs` into `out` (zeros the unused tail).
pub fn encode(blobs: &[Blob<'_>], out: &mut [u8]) -> Result<usize, EncodeError> {
    if out.len() < 16 {
        return Err(EncodeError::TooLarge);
    }
    if blobs.len() > MAX_BLOBS {
        return Err(EncodeError::TooLarge);
    }
    out.fill(0);
    write_u32(out, 0, MAGIC)?;
    write_u32(out, 4, VERSION)?;
    write_u32(out, 8, blobs.len() as u32)?;
    write_u32(out, 12, 0)?;
    let mut off = 16usize;
    for b in blobs {
        let klen = b.key.len();
        if klen == 0 || klen > MAX_KEY_LEN {
            return Err(EncodeError::BadKey);
        }
        if b.payload.len() > MAX_PAYLOAD {
            return Err(EncodeError::TooLarge);
        }
        let need = klen + 1 + 2 + b.payload.len();
        let end = off.checked_add(need).ok_or(EncodeError::TooLarge)?;
        if end > out.len() {
            return Err(EncodeError::TooLarge);
        }
        out[off] = klen as u8;
        off += 1;
        out[off..off + klen].copy_from_slice(b.key);
        off += klen;
        let plen = b.payload.len() as u16;
        out[off] = (plen & 0xff) as u8;
        out[off + 1] = (plen >> 8) as u8;
        off += 2;
        out[off..off + b.payload.len()].copy_from_slice(b.payload);
        off += b.payload.len();
    }
    Ok(off)
}

/// Decode into `keys`/`payloads` scratch owned by the caller; returns count.
pub fn decode<'a>(
    buf: &'a [u8],
    keys: &'a mut [[u8; MAX_KEY_LEN]; MAX_BLOBS],
    key_lens: &mut [usize; MAX_BLOBS],
    payloads: &'a mut [[u8; MAX_PAYLOAD]; MAX_BLOBS],
    payload_lens: &mut [usize; MAX_BLOBS],
) -> Result<usize, DecodeError> {
    if buf.len() < 16 {
        return Err(DecodeError::TooShort);
    }
    if read_u32(buf, 0)? != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    if read_u32(buf, 4)? != VERSION {
        return Err(DecodeError::BadVersion);
    }
    let count = read_u32(buf, 8)? as usize;
    if count > MAX_BLOBS {
        return Err(DecodeError::BadCount);
    }
    let mut off = 16usize;
    for i in 0..count {
        let klen = *buf.get(off).ok_or(DecodeError::Truncated)? as usize;
        off += 1;
        if klen == 0 || klen > MAX_KEY_LEN {
            return Err(DecodeError::BadKey);
        }
        let kend = off.checked_add(klen).ok_or(DecodeError::Truncated)?;
        let key = buf.get(off..kend).ok_or(DecodeError::Truncated)?;
        keys[i][..klen].copy_from_slice(key);
        key_lens[i] = klen;
        off = kend;
        let lo = *buf.get(off).ok_or(DecodeError::Truncated)? as usize;
        let hi = *buf.get(off + 1).ok_or(DecodeError::Truncated)? as usize;
        off += 2;
        let plen = lo | (hi << 8);
        if plen > MAX_PAYLOAD {
            return Err(DecodeError::Truncated);
        }
        let pend = off.checked_add(plen).ok_or(DecodeError::Truncated)?;
        let payload = buf.get(off..pend).ok_or(DecodeError::Truncated)?;
        payloads[i][..plen].copy_from_slice(payload);
        payload_lens[i] = plen;
        off = pend;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let mut region = [0u8; REGION_SIZE];
        let blobs = [
            Blob {
                key: b"cfg",
                payload: b"harbor",
            },
            Blob {
                key: b"n",
                payload: b"1",
            },
        ];
        encode(&blobs, &mut region).unwrap();
        let mut keys = [[0u8; MAX_KEY_LEN]; MAX_BLOBS];
        let mut kl = [0usize; MAX_BLOBS];
        let mut payloads = [[0u8; MAX_PAYLOAD]; MAX_BLOBS];
        let mut pl = [0usize; MAX_BLOBS];
        let n = decode(&region, &mut keys, &mut kl, &mut payloads, &mut pl).unwrap();
        assert_eq!(n, 2);
        assert_eq!(&keys[0][..kl[0]], b"cfg");
        assert_eq!(&payloads[0][..pl[0]], b"harbor");
        assert_eq!(&keys[1][..kl[1]], b"n");
    }

    #[test]
    fn bad_magic_refused() {
        let mut region = [0u8; 32];
        region[0] = b'X';
        let mut keys = [[0u8; MAX_KEY_LEN]; MAX_BLOBS];
        let mut kl = [0usize; MAX_BLOBS];
        let mut payloads = [[0u8; MAX_PAYLOAD]; MAX_BLOBS];
        let mut pl = [0usize; MAX_BLOBS];
        assert_eq!(
            decode(&region, &mut keys, &mut kl, &mut payloads, &mut pl),
            Err(DecodeError::BadMagic)
        );
    }

    #[test]
    fn empty_key_refused() {
        let mut region = [0u8; REGION_SIZE];
        assert_eq!(
            encode(
                &[Blob {
                    key: b"",
                    payload: b"x"
                }],
                &mut region
            ),
            Err(EncodeError::BadKey)
        );
    }
}
