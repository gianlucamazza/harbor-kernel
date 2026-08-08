//! Durable store façade (ADR-0045 / P2 residual).
//!
//! Region: `.durable_store` in the image (NOLOAD, not BSS-cleared). Pure
//! encode/decode lives in [`kernel_core::durable`].

use kernel_core::durable::{self, Blob, DecodeError, EncodeError, REGION_SIZE};

// Linker window (ADR-0045). Not `static mut` (rule 7 / ADR-0019): address only.
unsafe extern "C" {
    safe static __durable_store_start: u8;
    safe static __durable_store_end: u8;
}

/// Run `f` over the durable window; the borrow ends with the closure.
///
/// This used to be `fn region() -> &'static mut [u8]` — an unbounded aliasable
/// `&mut` from an innocent-looking call, the exact shape the no-static-mut
/// gate cannot see (excellence review F-26). Scoping the borrow to a closure
/// makes two overlapping calls impossible to write, and is the SMP-ready
/// shape: at K8 this runner becomes a lock acquisition and no call site moves.
fn with_region<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    let start = core::ptr::addr_of!(__durable_store_start) as *mut u8;
    let end = core::ptr::addr_of!(__durable_store_end) as usize;
    let len = end.saturating_sub(start as usize).min(REGION_SIZE);
    // SAFETY: single-core; exclusive durable writer; region is RW-mapped data;
    // the borrow is scoped to `f`, so no second `&mut` can coexist.
    f(unsafe { core::slice::from_raw_parts_mut(start, len) })
}

/// Put `key`/`payload` into the durable region (read-modify-write).
pub fn put(key: &[u8], payload: &[u8]) -> Result<(), EncodeError> {
    let mut keys = [[0u8; durable::MAX_KEY_LEN]; durable::MAX_BLOBS];
    let mut kl = [0usize; durable::MAX_BLOBS];
    let mut payloads = [[0u8; durable::MAX_PAYLOAD]; durable::MAX_BLOBS];
    let mut pl = [0usize; durable::MAX_BLOBS];
    let mut n =
        match with_region(|r| durable::decode(r, &mut keys, &mut kl, &mut payloads, &mut pl)) {
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
    with_region(|r| durable::encode(&blobs[..n], r).map(|_| ()))
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
