//! External agent store wire format (ADR-0027) — pure, host-tested.
//!
//! A packed composition of agents that may sit outside kernel `.rodata`. The
//! loader maps a physical range and calls [`parse`]; no MMIO lives here.

use crate::manifest::{AgentEntry, MAX_SLOTS};

/// `b"HARB"` little-endian.
pub const MAGIC: u32 = u32::from_le_bytes(*b"HARB");

/// Format version accepted by this parser.
pub const VERSION: u32 = 1;

/// Hard cap on agents in one store (matches lab `MAX_TASKS` headroom).
pub const MAX_AGENTS: usize = 8;

/// Fixed name field width (UTF-8, NUL-padded).
pub const NAME_LEN: usize = 16;

/// Empty slot marker in the wire format.
pub const SLOT_NONE: u8 = 0xFF;

/// Why a store could not be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    TooShort,
    BadMagic,
    BadVersion,
    BadCount,
    Truncated,
    BadName,
    BadGeometry,
    ImageTooLarge,
}

/// One agent record borrowing image bytes from the store blob.
#[derive(Clone, Copy, Debug)]
pub struct StoreAgent<'a> {
    pub name: &'a [u8],
    pub text_pages: u32,
    pub stack_pages: u32,
    pub slots: [u8; MAX_SLOTS],
    pub image: &'a [u8],
}

/// A validated store.
#[derive(Clone, Copy, Debug)]
pub struct Store<'a> {
    pub agents: &'a [StoreAgent<'a>],
}

fn read_u32(buf: &[u8], off: usize) -> Result<u32, ParseError> {
    let end = off.checked_add(4).ok_or(ParseError::Truncated)?;
    let s = buf.get(off..end).ok_or(ParseError::Truncated)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Parse a store blob. On success, `out` is filled with up to [`MAX_AGENTS`]
/// records and the returned slice is a prefix of `out`.
pub fn parse<'a>(
    buf: &'a [u8],
    out: &'a mut [StoreAgent<'a>; MAX_AGENTS],
) -> Result<&'a [StoreAgent<'a>], ParseError> {
    if buf.len() < 16 {
        return Err(ParseError::TooShort);
    }
    if read_u32(buf, 0)? != MAGIC {
        return Err(ParseError::BadMagic);
    }
    if read_u32(buf, 4)? != VERSION {
        return Err(ParseError::BadVersion);
    }
    let count = read_u32(buf, 8)? as usize;
    if count == 0 || count > MAX_AGENTS {
        return Err(ParseError::BadCount);
    }
    let _reserved = read_u32(buf, 12)?;

    let mut off = 16usize;
    for i in 0..count {
        let name_end = off.checked_add(NAME_LEN).ok_or(ParseError::Truncated)?;
        let name = buf.get(off..name_end).ok_or(ParseError::Truncated)?;
        off = name_end;

        // Name must be UTF-8 with at least one non-NUL byte before padding.
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
        if name_len == 0 {
            return Err(ParseError::BadName);
        }
        if core::str::from_utf8(&name[..name_len]).is_err() {
            return Err(ParseError::BadName);
        }

        let text_pages = read_u32(buf, off)?;
        off += 4;
        let stack_pages = read_u32(buf, off)?;
        off += 4;
        if text_pages == 0 || stack_pages == 0 {
            return Err(ParseError::BadGeometry);
        }

        let slots_end = off.checked_add(MAX_SLOTS).ok_or(ParseError::Truncated)?;
        let slots_raw = buf.get(off..slots_end).ok_or(ParseError::Truncated)?;
        let mut slots = [SLOT_NONE; MAX_SLOTS];
        slots.copy_from_slice(slots_raw);
        off = slots_end;

        off = off.checked_add(4).ok_or(ParseError::Truncated)?; // reserved

        let image_len = read_u32(buf, off)? as usize;
        off += 4;
        let img_end = off.checked_add(image_len).ok_or(ParseError::Truncated)?;
        let image = buf.get(off..img_end).ok_or(ParseError::Truncated)?;
        off = align4(img_end);

        let capacity = (text_pages as usize).saturating_mul(4096);
        if image_len > capacity {
            return Err(ParseError::ImageTooLarge);
        }

        out[i] = StoreAgent {
            name: &name[..name_len],
            text_pages,
            stack_pages,
            slots,
            image,
        };
    }

    Ok(&out[..count])
}

/// Pack one agent into a growing buffer (host / packer helper).
#[cfg(test)]
pub fn append_agent(
    buf: &mut Vec<u8>,
    name: &str,
    text_pages: u32,
    stack_pages: u32,
    slots: [u8; MAX_SLOTS],
    image: &[u8],
) {
    let mut name_field = [0u8; NAME_LEN];
    let nb = name.as_bytes();
    let n = nb.len().min(NAME_LEN);
    name_field[..n].copy_from_slice(&nb[..n]);
    buf.extend_from_slice(&name_field);
    buf.extend_from_slice(&text_pages.to_le_bytes());
    buf.extend_from_slice(&stack_pages.to_le_bytes());
    buf.extend_from_slice(&slots);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&(image.len() as u32).to_le_bytes());
    buf.extend_from_slice(image);
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

/// Build a complete store blob (host / packer helper).
#[cfg(test)]
pub fn pack(agents: &[(/*name*/ &str, u32, u32, [u8; MAX_SLOTS], &[u8])]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&MAGIC.to_le_bytes());
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&(agents.len() as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    for (name, tp, sp, slots, image) in agents {
        append_agent(&mut buf, name, *tp, *sp, *slots, image);
    }
    buf
}

/// Convert a store agent into a manifest entry, using caller-provided
/// `'static` name and image storage that outlive the entry.
pub fn to_entry(agent: &StoreAgent<'_>, name: &'static str, image: &'static [u8]) -> AgentEntry {
    let mut slots = [None; MAX_SLOTS];
    for (i, &s) in agent.slots.iter().enumerate() {
        if s != SLOT_NONE {
            slots[i] = Some(s);
        }
    }
    AgentEntry {
        name,
        image,
        text_pages: agent.text_pages as usize,
        stack_pages: agent.stack_pages as usize,
        slots,
        device: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prog;

    #[test]
    fn round_trip_beacon_shaped_agent() {
        let image = prog::encode_console_hi_exit(1);
        let mut slots = [SLOT_NONE; MAX_SLOTS];
        slots[1] = 0; // held console at index 0
        let blob = pack(&[("beacon", 1, 3, slots, &image)]);

        let mut out = [StoreAgent {
            name: b"",
            text_pages: 0,
            stack_pages: 0,
            slots: [SLOT_NONE; MAX_SLOTS],
            image: b"",
        }; MAX_AGENTS];
        let agents = parse(&blob, &mut out).expect("parse");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, b"beacon");
        assert_eq!(agents[0].text_pages, 1);
        assert_eq!(agents[0].stack_pages, 3);
        assert_eq!(agents[0].slots[1], 0);
        assert_eq!(agents[0].image, &image);
    }

    #[test]
    fn bad_magic_is_refused() {
        let mut blob = pack(&[("x", 1, 1, [SLOT_NONE; MAX_SLOTS], &[0u8; 4])]);
        blob[0] = b'X';
        let mut out = [StoreAgent {
            name: b"",
            text_pages: 0,
            stack_pages: 0,
            slots: [SLOT_NONE; MAX_SLOTS],
            image: b"",
        }; MAX_AGENTS];
        assert!(matches!(parse(&blob, &mut out), Err(ParseError::BadMagic)));
    }

    #[test]
    fn empty_count_is_refused() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&MAGIC.to_le_bytes());
        blob.extend_from_slice(&VERSION.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());
        let mut out = [StoreAgent {
            name: b"",
            text_pages: 0,
            stack_pages: 0,
            slots: [SLOT_NONE; MAX_SLOTS],
            image: b"",
        }; MAX_AGENTS];
        assert!(matches!(parse(&blob, &mut out), Err(ParseError::BadCount)));
    }
}
