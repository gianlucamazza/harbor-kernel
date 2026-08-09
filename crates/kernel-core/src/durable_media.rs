//! Media wrapper for the durable store (ADR-0066) — pure, host-testable.
//!
//! The [`crate::durable`] DURB block is the *content* format; this module
//! owns how 4 KiB of it sits on SD sectors so a power cut mid-flush loses
//! at most the newest write, never the last good state: two slots, a
//! header sector per slot carrying a sequence number and a CRC32 of the
//! payload, and the rule that the header is written **last** (the commit).
//!
//! Load validates both slots and takes the highest valid sequence. The
//! driver executes reads/writes; every decision — winner, next slot,
//! header bytes — is made here where a host test can reach it.

use crate::durable::REGION_SIZE;

/// SD sector size this layout is written in.
pub const SECTOR_SIZE: usize = 512;

/// Payload sectors per slot (the 4 KiB DURB block).
pub const PAYLOAD_SECTORS: usize = REGION_SIZE / SECTOR_SIZE;

/// Sector offsets inside the store partition.
pub const SLOT_A_HEADER: u32 = 0;
pub const SLOT_A_PAYLOAD: u32 = 1;
pub const SLOT_B_HEADER: u32 = 16;
pub const SLOT_B_PAYLOAD: u32 = 17;

/// Smallest partition this layout fits in (sectors). A 1 MiB partition is
/// 2048 — far above; the guard exists for a hand-shrunk entry.
pub const MIN_PARTITION_SECTORS: u32 = 32;

/// Header magic: "DMH1".
pub const MAGIC: u32 = u32::from_le_bytes(*b"DMH1");
pub const VERSION: u32 = 1;

/// One slot of the double buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    #[inline]
    pub const fn header_sector(self) -> u32 {
        match self {
            Slot::A => SLOT_A_HEADER,
            Slot::B => SLOT_B_HEADER,
        }
    }

    #[inline]
    pub const fn payload_sector(self) -> u32 {
        match self {
            Slot::A => SLOT_A_PAYLOAD,
            Slot::B => SLOT_B_PAYLOAD,
        }
    }
}

/// The slot the next flush must write: always the one **not** holding the
/// current good state, so a torn write cannot destroy it.
#[inline]
pub const fn next_slot(winner: Option<Slot>) -> Slot {
    match winner {
        None | Some(Slot::B) => Slot::A,
        Some(Slot::A) => Slot::B,
    }
}

/// Decoded, validated header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub seq: u64,
    pub crc: u32,
}

/// CRC32 (IEEE 802.3, reflected, init/xorout `0xFFFF_FFFF`) — bitwise, no
/// table: the inputs are 4 KiB once per boot, not a datapath.
pub const fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    let mut i = 0;
    while i < bytes.len() {
        crc ^= bytes[i] as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = if crc & 1 != 0 { 0xEDB8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
            bit += 1;
        }
        i += 1;
    }
    !crc
}

/// Encode a header sector committing `payload` at sequence `seq`.
pub fn encode_header(seq: u64, payload: &[u8; REGION_SIZE]) -> [u8; SECTOR_SIZE] {
    let mut out = [0u8; SECTOR_SIZE];
    out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    out[4..8].copy_from_slice(&VERSION.to_le_bytes());
    out[8..16].copy_from_slice(&seq.to_le_bytes());
    out[16..20].copy_from_slice(&crc32(payload).to_le_bytes());
    out
}

/// Decode a header sector; `None` for anything but this exact format.
pub fn decode_header(sector: &[u8; SECTOR_SIZE]) -> Option<Header> {
    let magic = u32::from_le_bytes([sector[0], sector[1], sector[2], sector[3]]);
    let version = u32::from_le_bytes([sector[4], sector[5], sector[6], sector[7]]);
    if magic != MAGIC || version != VERSION {
        return None;
    }
    let seq = u64::from_le_bytes([
        sector[8], sector[9], sector[10], sector[11], sector[12], sector[13], sector[14],
        sector[15],
    ]);
    let crc = u32::from_le_bytes([sector[16], sector[17], sector[18], sector[19]]);
    Some(Header { seq, crc })
}

/// A slot is valid when its header decodes and commits exactly its payload.
pub fn validate(header: &[u8; SECTOR_SIZE], payload: &[u8; REGION_SIZE]) -> Option<Header> {
    let h = decode_header(header)?;
    if crc32(payload) != h.crc {
        return None;
    }
    Some(h)
}

/// Pick the good state among two read-back slots: highest valid sequence.
/// `None` → fresh media (empty store).
pub fn pick_winner(
    header_a: &[u8; SECTOR_SIZE],
    payload_a: &[u8; REGION_SIZE],
    header_b: &[u8; SECTOR_SIZE],
    payload_b: &[u8; REGION_SIZE],
) -> Option<(Slot, Header)> {
    let a = validate(header_a, payload_a);
    let b = validate(header_b, payload_b);
    match (a, b) {
        (None, None) => None,
        (Some(h), None) => Some((Slot::A, h)),
        (None, Some(h)) => Some((Slot::B, h)),
        (Some(ha), Some(hb)) => {
            if ha.seq >= hb.seq {
                Some((Slot::A, ha))
            } else {
                Some((Slot::B, hb))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(fill: u8) -> [u8; REGION_SIZE] {
        [fill; REGION_SIZE]
    }

    #[test]
    fn crc32_matches_the_ieee_reference() {
        // The classic check value for "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn header_round_trips() {
        let p = payload(0x5A);
        let sector = encode_header(7, &p);
        let h = decode_header(&sector).unwrap();
        assert_eq!(h.seq, 7);
        assert_eq!(h.crc, crc32(&p));
        assert_eq!(validate(&sector, &p), Some(h));
    }

    #[test]
    fn fresh_media_has_no_winner() {
        let z = [0u8; SECTOR_SIZE];
        assert_eq!(pick_winner(&z, &payload(0), &z, &payload(0)), None);
        assert_eq!(next_slot(None), Slot::A, "first flush lands in A");
    }

    #[test]
    fn single_valid_slot_wins_and_flush_goes_to_the_other() {
        let p = payload(1);
        let ha = encode_header(1, &p);
        let z = [0u8; SECTOR_SIZE];
        let (slot, h) = pick_winner(&ha, &p, &z, &payload(0)).unwrap();
        assert_eq!((slot, h.seq), (Slot::A, 1));
        assert_eq!(next_slot(Some(slot)), Slot::B);
    }

    #[test]
    fn higher_sequence_wins_between_two_valid_slots() {
        let pa = payload(1);
        let pb = payload(2);
        let ha = encode_header(3, &pa);
        let hb = encode_header(4, &pb);
        let (slot, h) = pick_winner(&ha, &pa, &hb, &pb).unwrap();
        assert_eq!((slot, h.seq), (Slot::B, 4));
        assert_eq!(next_slot(Some(slot)), Slot::A, "alternation continues");
    }

    #[test]
    fn torn_payload_falls_back_to_the_other_slot() {
        // B committed seq 4 but its payload never fully landed: CRC saves us.
        let pa = payload(1);
        let ha = encode_header(3, &pa);
        let hb = encode_header(4, &payload(2));
        let torn = payload(0xEE);
        let (slot, h) = pick_winner(&ha, &pa, &hb, &torn).unwrap();
        assert_eq!((slot, h.seq), (Slot::A, 3), "previous good state survives");
    }

    #[test]
    fn torn_header_falls_back_to_the_other_slot() {
        let pa = payload(1);
        let ha = encode_header(3, &pa);
        let mut torn = encode_header(4, &payload(2));
        torn[0] = 0; // magic destroyed mid-write
        let (slot, _) = pick_winner(&ha, &pa, &torn, &payload(2)).unwrap();
        assert_eq!(slot, Slot::A);
    }

    #[test]
    fn geometry_stays_inside_the_minimum_partition() {
        assert!(SLOT_B_PAYLOAD + PAYLOAD_SECTORS as u32 <= MIN_PARTITION_SECTORS);
        assert_eq!(PAYLOAD_SECTORS, 8, "4 KiB DURB block");
    }
}
