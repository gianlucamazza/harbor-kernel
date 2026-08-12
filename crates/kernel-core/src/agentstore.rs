//! External agent store wire format (ADR-0027) — pure, host-tested.
//!
//! A packed composition of agents that may sit outside kernel `.rodata`. The
//! loader maps a physical range and calls [`parse`]; no MMIO lives here.

use crate::manifest::{AgentEntry, DeviceGrant, MAX_SLOTS};

/// `b"HARB"` little-endian.
pub const MAGIC: u32 = u32::from_le_bytes(*b"HARB");

/// Format version accepted by this parser.
///
/// **2** since ADR-0100 added the device window fields. Version 1 is refused
/// rather than accepted-with-defaults: no store in existence carries a device
/// grant, so there is no compatibility to keep, and a parser that guessed at a
/// missing field would be guessing about authority.
pub const VERSION: u32 = 2;

/// Hard cap on agents in one store.
///
/// It is a **wire-format** bound, not a scheduler one: the loader materialises
/// store entries into `[_; MAX_AGENTS]` pools that live for the boot, and this
/// is their size. It has no relation to `MAX_TASKS` — the comment here used to
/// claim it matched "lab `MAX_TASKS` headroom", which stopped being true the
/// first time that number moved (it is 54 today) and was never the reason for
/// this one. A composition that needs more agents raises this and the pools
/// with it; a composition that needs more *tasks* does not touch it.
pub const MAX_AGENTS: usize = 8;

/// Fixed name field width (UTF-8, NUL-padded).
pub const NAME_LEN: usize = 16;

/// Empty slot marker in the wire format.
pub const SLOT_NONE: u8 = 0xFF;

/// "No device window" marker in the wire format (ADR-0100).
///
/// Its own constant rather than a reuse of [`SLOT_NONE`]: the two index
/// different vocabularies, and a format where one sentinel means two things is
/// a format where a future change to one silently moves the other.
pub const WINDOW_NONE: u8 = 0xFF;

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
    /// Reserved word high bytes non-zero, or `home_cpu` ≥ `N_CPUS` (ADR-0088).
    BadHome,
    /// The device word's high bytes are non-zero, a window index came with an
    /// unaligned virtual address, or [`WINDOW_NONE`] came with a non-zero one
    /// (ADR-0100).
    ///
    /// That last one matters more than it looks: an entry that names no window
    /// has no use for an address, so a non-zero one means the record is not
    /// what it claims — and refusing it keeps the format from carrying an
    /// address nobody reads, which is where a future `pa` would try to live.
    BadWindow,
}

/// One agent record borrowing image bytes from the store blob.
#[derive(Clone, Copy, Debug)]
pub struct StoreAgent<'a> {
    pub name: &'a [u8],
    pub text_pages: u32,
    pub stack_pages: u32,
    pub slots: [u8; MAX_SLOTS],
    /// Sticky home CPU (ADR-0088); low byte of the wire reserved word.
    pub home_cpu: u8,
    /// Index into the loader's window vocabulary, or [`WINDOW_NONE`]
    /// (ADR-0100). An **index** — the wire has no field for a physical address,
    /// and that absence is the security property, not an omission.
    pub window: u8,
    /// Where the window lands in the agent's own address space. Meaningless
    /// unless `window` names one, and required to be zero when it does not.
    pub device_va: u64,
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

fn read_u64(buf: &[u8], off: usize) -> Result<u64, ParseError> {
    let end = off.checked_add(8).ok_or(ParseError::Truncated)?;
    let s = buf.get(off..end).ok_or(ParseError::Truncated)?;
    let mut b = [0u8; 8];
    b.copy_from_slice(s);
    Ok(u64::from_le_bytes(b))
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
    for entry in out.iter_mut().take(count) {
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

        // ADR-0088: reserved u32 — bits 7:0 = home_cpu; 31:8 must be zero.
        let reserved = read_u32(buf, off)?;
        off = off.checked_add(4).ok_or(ParseError::Truncated)?;
        if reserved & !0xff != 0 {
            return Err(ParseError::BadHome);
        }
        let home_cpu = (reserved & 0xff) as u8;
        if (home_cpu as usize) >= crate::tasks::N_CPUS {
            return Err(ParseError::BadHome);
        }

        // ADR-0100: device word — bits 7:0 = window index (or WINDOW_NONE);
        // 31:8 must be zero. Then the virtual address it lands at. There is no
        // physical address on the wire, at any width: an entry names a position
        // in the board's vocabulary, and the board says what that page is.
        let device_word = read_u32(buf, off)?;
        off = off.checked_add(4).ok_or(ParseError::Truncated)?;
        if device_word & !0xff != 0 {
            return Err(ParseError::BadWindow);
        }
        let window = (device_word & 0xff) as u8;
        let device_va = read_u64(buf, off)?;
        off = off.checked_add(8).ok_or(ParseError::Truncated)?;
        if window == WINDOW_NONE {
            // No window, no address. Refused rather than ignored, so the format
            // cannot quietly carry an address that nothing reads.
            if device_va != 0 {
                return Err(ParseError::BadWindow);
            }
        } else if !device_va.is_multiple_of(4096) {
            return Err(ParseError::BadWindow);
        }

        let image_len = read_u32(buf, off)? as usize;
        off += 4;
        let img_end = off.checked_add(image_len).ok_or(ParseError::Truncated)?;
        let image = buf.get(off..img_end).ok_or(ParseError::Truncated)?;
        off = align4(img_end);

        let capacity = (text_pages as usize).saturating_mul(4096);
        if image_len > capacity {
            return Err(ParseError::ImageTooLarge);
        }

        *entry = StoreAgent {
            name: &name[..name_len],
            text_pages,
            stack_pages,
            slots,
            home_cpu,
            window,
            device_va,
            image,
        };
    }

    Ok(&out[..count])
}

/// Pack one agent into a growing buffer (host / packer helper).
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn append_agent(
    buf: &mut Vec<u8>,
    name: &str,
    text_pages: u32,
    stack_pages: u32,
    slots: [u8; MAX_SLOTS],
    home_cpu: u8,
    window: u8,
    device_va: u64,
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
    buf.extend_from_slice(&(home_cpu as u32).to_le_bytes());
    buf.extend_from_slice(&(window as u32).to_le_bytes());
    buf.extend_from_slice(&device_va.to_le_bytes());
    buf.extend_from_slice(&(image.len() as u32).to_le_bytes());
    buf.extend_from_slice(image);
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

/// One agent argument to [`pack`] (host / packer helper).
///
/// No device fields: an agent with a window is the exception, and a test that
/// wants one calls [`append_agent`] directly rather than making every other
/// caller carry two more tuple members.
#[cfg(test)]
type PackAgent<'a> = (&'a str, u32, u32, [u8; MAX_SLOTS], u8, &'a [u8]);

/// Build a complete store blob (host / packer helper).
#[cfg(test)]
pub fn pack(agents: &[PackAgent<'_>]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&MAGIC.to_le_bytes());
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&(agents.len() as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    for (name, tp, sp, slots, home, image) in agents {
        append_agent(
            &mut buf,
            name,
            *tp,
            *sp,
            *slots,
            *home,
            WINDOW_NONE,
            0,
            image,
        );
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
        device: (agent.window != WINDOW_NONE).then_some(DeviceGrant {
            va: agent.device_va,
            window: agent.window,
        }),
        home_cpu: agent.home_cpu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prog;

    fn empty_slot() -> StoreAgent<'static> {
        StoreAgent {
            name: b"",
            text_pages: 0,
            stack_pages: 0,
            slots: [SLOT_NONE; MAX_SLOTS],
            home_cpu: 0,
            window: WINDOW_NONE,
            device_va: 0,
            image: b"",
        }
    }

    #[test]
    fn round_trip_beacon_shaped_agent() {
        let image = prog::encode_console_hi_exit(1);
        let mut slots = [SLOT_NONE; MAX_SLOTS];
        slots[1] = 0; // held console at index 0
        let blob = pack(&[("beacon", 1, 3, slots, 0, &image)]);

        let mut out = [empty_slot(); MAX_AGENTS];
        let agents = parse(&blob, &mut out).expect("parse");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, b"beacon");
        assert_eq!(agents[0].text_pages, 1);
        assert_eq!(agents[0].stack_pages, 3);
        assert_eq!(agents[0].slots[1], 0);
        assert_eq!(agents[0].home_cpu, 0);
        assert_eq!(agents[0].image, &image);
    }

    #[test]
    fn home_cpu_round_trips_in_reserved_word() {
        let image = prog::encode_console_hi_exit(1);
        let mut slots = [SLOT_NONE; MAX_SLOTS];
        slots[1] = 0;
        let blob = pack(&[("chirp", 1, 3, slots, 1, &image)]);
        let mut out = [empty_slot(); MAX_AGENTS];
        let agents = parse(&blob, &mut out).expect("parse");
        assert_eq!(agents[0].home_cpu, 1);
        assert_eq!(agents[0].name, b"chirp");
    }

    /// Pack one agent that asks for a device window (ADR-0100).
    fn pack_with_window(window: u8, device_va: u64, image: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC.to_le_bytes());
        buf.extend_from_slice(&VERSION.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        append_agent(
            &mut buf,
            "driver",
            1,
            3,
            [SLOT_NONE; MAX_SLOTS],
            0,
            window,
            device_va,
            image,
        );
        buf
    }

    #[test]
    fn a_window_index_and_its_va_round_trip() {
        let image = prog::encode_console_hi_exit(1);
        let blob = pack_with_window(1, 0x9000, &image);
        let mut out = [empty_slot(); MAX_AGENTS];
        let agents = parse(&blob, &mut out).expect("parse");
        assert_eq!(agents[0].window, 1);
        assert_eq!(agents[0].device_va, 0x9000);

        // …and reaches the manifest entry as a grant naming the same position.
        static IMG: [u8; 4] = [0; 4];
        let entry = to_entry(&agents[0], "driver", &IMG);
        assert_eq!(
            entry.device,
            Some(DeviceGrant {
                va: 0x9000,
                window: 1
            })
        );
    }

    #[test]
    fn an_entry_with_no_window_carries_no_address() {
        // The format cannot quietly hold an address nothing reads — which is
        // where a physical one would try to live if it ever came back.
        let image = prog::encode_console_hi_exit(1);
        let blob = pack_with_window(WINDOW_NONE, 0x9000, &image);
        let mut out = [empty_slot(); MAX_AGENTS];
        assert!(matches!(parse(&blob, &mut out), Err(ParseError::BadWindow)));

        // `parse` borrows `out` for as long as the records live, so the second
        // half of the story needs its own buffer.
        let ok = pack_with_window(WINDOW_NONE, 0, &image);
        let mut out2 = [empty_slot(); MAX_AGENTS];
        let agents = parse(&ok, &mut out2).expect("parse");
        assert_eq!(agents[0].window, WINDOW_NONE);
        static IMG: [u8; 4] = [0; 4];
        assert_eq!(to_entry(&agents[0], "driver", &IMG).device, None);
    }

    #[test]
    fn an_unaligned_window_address_is_refused() {
        // A device page is a page. An unaligned VA would be rejected later by
        // the address space anyway; refusing it here keeps the failure in the
        // layer that owns the format.
        let image = prog::encode_console_hi_exit(1);
        let blob = pack_with_window(0, 0x9001, &image);
        let mut out = [empty_slot(); MAX_AGENTS];
        assert!(matches!(parse(&blob, &mut out), Err(ParseError::BadWindow)));
    }

    #[test]
    fn the_device_word_has_no_room_for_anything_but_an_index() {
        // 31:8 reserved and checked, exactly as ADR-0088 did for `home_cpu`.
        // A future field that tried to smuggle itself into those bits fails
        // here rather than being read as part of the index.
        let image = prog::encode_console_hi_exit(1);
        let mut blob = pack_with_window(0, 0x9000, &image);
        // The device word sits after name(16) + text(4) + stack(4) + slots(4)
        // + reserved(4), inside the record that starts at 16.
        let off = 16 + NAME_LEN + 4 + 4 + MAX_SLOTS + 4;
        blob[off + 1] = 0x01;
        let mut out = [empty_slot(); MAX_AGENTS];
        assert!(matches!(parse(&blob, &mut out), Err(ParseError::BadWindow)));
    }

    #[test]
    fn version_one_is_refused_rather_than_read_with_defaults() {
        // ADR-0100 changed the record, so a v1 blob is not a v2 blob missing a
        // field — its image offset is different, and reading it as v2 would
        // parse image bytes as a device word. Refusing is the only honest
        // answer, and no store in existence is v1 anyway.
        let image = prog::encode_console_hi_exit(1);
        let mut blob = pack(&[("beacon", 1, 3, [SLOT_NONE; MAX_SLOTS], 0, &image)]);
        blob[4..8].copy_from_slice(&1u32.to_le_bytes());
        let mut out = [empty_slot(); MAX_AGENTS];
        assert!(matches!(
            parse(&blob, &mut out),
            Err(ParseError::BadVersion)
        ));
    }

    #[test]
    fn home_cpu_out_of_range_is_refused() {
        let image = [0u8; 4];
        let blob = pack(&[("x", 1, 1, [SLOT_NONE; MAX_SLOTS], 2, &image)]);
        let mut out = [empty_slot(); MAX_AGENTS];
        assert!(matches!(parse(&blob, &mut out), Err(ParseError::BadHome)));
    }

    #[test]
    fn bad_magic_is_refused() {
        let mut blob = pack(&[("x", 1, 1, [SLOT_NONE; MAX_SLOTS], 0, &[0u8; 4])]);
        blob[0] = b'X';
        let mut out = [empty_slot(); MAX_AGENTS];
        assert!(matches!(parse(&blob, &mut out), Err(ParseError::BadMagic)));
    }

    #[test]
    fn empty_count_is_refused() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&MAGIC.to_le_bytes());
        blob.extend_from_slice(&VERSION.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());
        let mut out = [empty_slot(); MAX_AGENTS];
        assert!(matches!(parse(&blob, &mut out), Err(ParseError::BadCount)));
    }
}
