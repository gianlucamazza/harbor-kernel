//! Durable store façade (ADR-0045 / P2 residual).
//!
//! Region: `.durable_store` in the image (NOLOAD, not BSS-cleared). Pure
//! encode/decode lives in [`kernel_core::durable`]. Access is serialised with
//! [`IrqSpinLock`] (ADR-0077).

use kernel_core::durable::{self, Blob, DecodeError, EncodeError, REGION_SIZE};

use crate::sync::IrqSpinLock;

// Linker window (ADR-0045). Not `static mut` (rule 7 / ADR-0019): address only.
unsafe extern "C" {
    safe static __durable_store_start: u8;
    safe static __durable_store_end: u8;
}

static DURABLE_LOCK: IrqSpinLock = IrqSpinLock::new();

/// Run `f` over the durable window under the durable lock; the borrow ends
/// with the closure (no aliasable long-lived `&mut` — excellence F-26).
fn with_region<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    DURABLE_LOCK.with(|| {
        let start = core::ptr::addr_of!(__durable_store_start) as *mut u8;
        let end = core::ptr::addr_of!(__durable_store_end) as usize;
        let len = end.saturating_sub(start as usize).min(REGION_SIZE);
        // SAFETY: exclusivity from DURABLE_LOCK; region is RW-mapped data;
        // the borrow is scoped to `f`.
        f(unsafe { core::slice::from_raw_parts_mut(start, len) })
    })
}

/// Put `key`/`payload` into the durable region (atomic read-modify-write).
pub fn put(key: &[u8], payload: &[u8]) -> Result<(), EncodeError> {
    with_region(|r| {
        let mut keys = [[0u8; durable::MAX_KEY_LEN]; durable::MAX_BLOBS];
        let mut kl = [0usize; durable::MAX_BLOBS];
        let mut payloads = [[0u8; durable::MAX_PAYLOAD]; durable::MAX_BLOBS];
        let mut pl = [0usize; durable::MAX_BLOBS];
        let mut n = match durable::decode(r, &mut keys, &mut kl, &mut payloads, &mut pl) {
            Ok(n) => n,
            Err(DecodeError::BadMagic | DecodeError::TooShort | DecodeError::BadVersion) => 0,
            Err(_) => 0,
        };
        // Replace or append.
        let mut found = false;
        for i in 0..n {
            if kl[i] == key.len() && keys[i][..kl[i]] == key[..] {
                if payload.len() > durable::MAX_PAYLOAD {
                    return Err(EncodeError::TooLarge);
                }
                payloads[i][..payload.len()].copy_from_slice(payload);
                pl[i] = payload.len();
                found = true;
                break;
            }
        }
        if !found {
            if n >= durable::MAX_BLOBS {
                return Err(EncodeError::TooLarge);
            }
            if key.is_empty() || key.len() > durable::MAX_KEY_LEN {
                return Err(EncodeError::BadKey);
            }
            if payload.len() > durable::MAX_PAYLOAD {
                return Err(EncodeError::TooLarge);
            }
            keys[n][..key.len()].copy_from_slice(key);
            kl[n] = key.len();
            payloads[n][..payload.len()].copy_from_slice(payload);
            pl[n] = payload.len();
            n += 1;
        }
        let mut blobs = [Blob {
            key: &[],
            payload: &[],
        }; durable::MAX_BLOBS];
        for i in 0..n {
            blobs[i] = Blob {
                key: &keys[i][..kl[i]],
                payload: &payloads[i][..pl[i]],
            };
        }
        durable::encode(&blobs[..n], r).map(|_| ())
    })
}

/// Copy the raw durable region out — the bytes a media flush commits
/// (ADR-0066). The caller owns what happens to them; this module keeps its
/// arch-only imports and never learns a driver exists.
pub fn snapshot() -> [u8; REGION_SIZE] {
    let mut out = [0u8; REGION_SIZE];
    with_region(|r| out[..r.len()].copy_from_slice(r));
    out
}

/// Overwrite the durable region with bytes loaded from media (ADR-0066).
///
/// Called by bootstrap before any `put` this boot, so a valid media image
/// is the store — the same "the section is the store" rule as ADR-0045,
/// with the section now seeded from the card.
pub fn restore(bytes: &[u8; REGION_SIZE]) {
    with_region(|r| {
        let len = r.len();
        r.copy_from_slice(&bytes[..len]);
    });
}

/// Read `key` from the durable region into `out`.
pub fn get(key: &[u8], out: &mut [u8]) -> Result<usize, DecodeError> {
    let mut keys = [[0u8; durable::MAX_KEY_LEN]; durable::MAX_BLOBS];
    let mut kl = [0usize; durable::MAX_BLOBS];
    let mut payloads = [[0u8; durable::MAX_PAYLOAD]; durable::MAX_BLOBS];
    let mut pl = [0usize; durable::MAX_BLOBS];
    let n = with_region(|r| durable::decode(r, &mut keys, &mut kl, &mut payloads, &mut pl))?;
    for i in 0..n {
        if kl[i] == key.len() && keys[i][..kl[i]] == key[..] {
            let len = pl[i];
            if out.len() < len {
                return Err(DecodeError::Truncated);
            }
            out[..len].copy_from_slice(&payloads[i][..len]);
            return Ok(len);
        }
    }
    Err(DecodeError::BadKey)
}
