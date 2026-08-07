//! Keyed blob store (ADR-0036 / P2) — pure, host-tested.
//!
//! Fixed-capacity put/get/delete of opaque payloads by short keys. Backing is
//! embedded in the table (no heap, no MMIO). Creators own policy for who may
//! call the EL1 façade.

/// Concurrent live blobs.
pub const MAX_BLOBS: usize = 4;

/// Max key bytes (no terminator required).
pub const MAX_KEY_LEN: usize = 16;

/// Max payload bytes per blob.
pub const MAX_PAYLOAD: usize = 64;

/// Why [`Table::put`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PutError {
    /// Empty key or longer than [`MAX_KEY_LEN`].
    BadKey,
    /// Payload longer than [`MAX_PAYLOAD`].
    TooLarge,
    /// Table full and key was not already present (replace would succeed).
    Full,
}

/// Why [`Table::get`] / [`Table::delete`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GetError {
    /// No blob for this key.
    Missing,
    /// Empty or too long key.
    BadKey,
    /// Caller buffer shorter than the stored payload.
    BufferTooSmall,
}

#[derive(Clone, Copy)]
struct Slot {
    live: bool,
    key_len: u8,
    key: [u8; MAX_KEY_LEN],
    payload_len: u16,
    payload: [u8; MAX_PAYLOAD],
}

impl Slot {
    const EMPTY: Self = Self {
        live: false,
        key_len: 0,
        key: [0; MAX_KEY_LEN],
        payload_len: 0,
        payload: [0; MAX_PAYLOAD],
    };
}

/// Pure keyed blob table.
#[derive(Clone)]
pub struct Table {
    slots: [Slot; MAX_BLOBS],
}

impl Table {
    pub const fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; MAX_BLOBS],
        }
    }

    /// Insert or replace `key` → `payload`.
    pub fn put(&mut self, key: &[u8], payload: &[u8]) -> Result<(), PutError> {
        let klen = validated_key_len(key).ok_or(PutError::BadKey)?;
        if payload.len() > MAX_PAYLOAD {
            return Err(PutError::TooLarge);
        }
        if let Some(i) = self.find(key) {
            self.write_slot(i, klen, key, payload);
            return Ok(());
        }
        for (i, s) in self.slots.iter().enumerate() {
            if !s.live {
                self.write_slot(i, klen, key, payload);
                return Ok(());
            }
        }
        Err(PutError::Full)
    }

    /// Copy the payload for `key` into `out`; returns the number of bytes written.
    pub fn get(&self, key: &[u8], out: &mut [u8]) -> Result<usize, GetError> {
        let _ = validated_key_len(key).ok_or(GetError::BadKey)?;
        let i = self.find(key).ok_or(GetError::Missing)?;
        let n = self.slots[i].payload_len as usize;
        if out.len() < n {
            return Err(GetError::BufferTooSmall);
        }
        out[..n].copy_from_slice(&self.slots[i].payload[..n]);
        Ok(n)
    }

    /// Remove a blob.
    pub fn delete(&mut self, key: &[u8]) -> Result<(), GetError> {
        let _ = validated_key_len(key).ok_or(GetError::BadKey)?;
        let i = self.find(key).ok_or(GetError::Missing)?;
        self.slots[i] = Slot::EMPTY;
        Ok(())
    }

    /// True if `key` is present.
    pub fn contains(&self, key: &[u8]) -> bool {
        self.find(key).is_some()
    }

    fn write_slot(&mut self, i: usize, klen: usize, key: &[u8], payload: &[u8]) {
        let s = &mut self.slots[i];
        s.live = true;
        s.key_len = klen as u8;
        s.key = [0; MAX_KEY_LEN];
        s.key[..klen].copy_from_slice(&key[..klen]);
        s.payload_len = payload.len() as u16;
        s.payload = [0; MAX_PAYLOAD];
        s.payload[..payload.len()].copy_from_slice(payload);
    }

    fn find(&self, key: &[u8]) -> Option<usize> {
        let len = key.len();
        for (i, s) in self.slots.iter().enumerate() {
            if s.live && s.key_len as usize == len && s.key[..len] == key[..] {
                return Some(i);
            }
        }
        None
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

fn validated_key_len(key: &[u8]) -> Option<usize> {
    let n = key.len();
    if n == 0 || n > MAX_KEY_LEN {
        None
    } else {
        Some(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_round_trip() {
        let mut t = Table::new();
        t.put(b"cfg", b"hello").unwrap();
        let mut out = [0u8; MAX_PAYLOAD];
        let n = t.get(b"cfg", &mut out).unwrap();
        assert_eq!(&out[..n], b"hello");
    }

    #[test]
    fn replace_updates_payload() {
        let mut t = Table::new();
        t.put(b"k", b"one").unwrap();
        t.put(b"k", b"two").unwrap();
        let mut out = [0u8; 8];
        let n = t.get(b"k", &mut out).unwrap();
        assert_eq!(&out[..n], b"two");
    }

    #[test]
    fn missing_is_refused() {
        let t = Table::new();
        let mut out = [0u8; 8];
        assert_eq!(t.get(b"nope", &mut out), Err(GetError::Missing));
        assert!(!t.contains(b"nope"));
    }

    #[test]
    fn delete_then_missing() {
        let mut t = Table::new();
        t.put(b"x", b"y").unwrap();
        t.delete(b"x").unwrap();
        let mut out = [0u8; 8];
        assert_eq!(t.get(b"x", &mut out), Err(GetError::Missing));
    }

    #[test]
    fn bad_key_and_too_large() {
        let mut t = Table::new();
        assert_eq!(t.put(b"", b"a"), Err(PutError::BadKey));
        assert_eq!(t.put(&[0u8; MAX_KEY_LEN + 1], b"a"), Err(PutError::BadKey));
        let big = [0u8; MAX_PAYLOAD + 1];
        assert_eq!(t.put(b"k", &big), Err(PutError::TooLarge));
        assert_eq!(t.get(b"", &mut [0u8; 1]), Err(GetError::BadKey));
    }

    #[test]
    fn full_table_refuses_new_key() {
        let mut t = Table::new();
        for i in 0..MAX_BLOBS {
            let key = [b'a' + i as u8];
            t.put(&key, b"v").unwrap();
        }
        assert_eq!(t.put(b"z", b"v"), Err(PutError::Full));
        // Replace still works.
        t.put(b"a", b"w").unwrap();
        let mut out = [0u8; 4];
        assert_eq!(t.get(b"a", &mut out).unwrap(), 1);
        assert_eq!(out[0], b'w');
    }

    #[test]
    fn short_buffer_is_refused() {
        let mut t = Table::new();
        t.put(b"k", b"abcdef").unwrap();
        let mut out = [0u8; 2];
        assert_eq!(t.get(b"k", &mut out), Err(GetError::BufferTooSmall));
    }

    #[test]
    fn keys_are_exact_match() {
        let mut t = Table::new();
        t.put(b"ab", b"1").unwrap();
        let mut out = [0u8; 4];
        assert_eq!(t.get(b"a", &mut out), Err(GetError::Missing));
        assert_eq!(t.get(b"abc", &mut out), Err(GetError::Missing));
        assert_eq!(t.get(b"ab", &mut out).unwrap(), 1);
    }
}
