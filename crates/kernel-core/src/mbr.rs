//! MBR partition-table reader (pure, host-testable) — ADR-0066.
//!
//! One decision lives here: **which LBA window on the card is the durable
//! store's**. The card's own partition table enumerates it — an entry of
//! type [`STORE_TYPE`] — so there is no magic block address anywhere in the
//! kernel, and the host tooling can verify the same table independently.
//!
//! Deliberately partial: four primary entries, no EBR chase, and a GPT
//! protective entry fails the lookup closed. Reading more table than the
//! decision needs would be parsing for parsing's sake.

/// MBR partition type designated for experimental use — the store's tag.
pub const STORE_TYPE: u8 = 0x7F;

/// GPT protective type: the MBR no longer describes the disk. Fail closed.
pub const GPT_PROTECTIVE: u8 = 0xEE;

/// One primary entry, as much of it as any caller decides on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartitionEntry {
    pub kind: u8,
    pub first_lba: u32,
    pub sectors: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MbrError {
    /// No `0x55AA` boot signature: not an MBR — nothing to trust or refuse.
    NoTable,
    /// A GPT protective entry is present: this table is a placeholder and
    /// the real layout is elsewhere. Refusing is the only honest answer.
    GptProtective,
}

const ENTRY_BASE: usize = 446;
const ENTRY_LEN: usize = 16;

/// Parse sector 0 into the four primary entries.
pub fn parse(sector0: &[u8; 512]) -> Result<[PartitionEntry; 4], MbrError> {
    if sector0[510] != 0x55 || sector0[511] != 0xAA {
        return Err(MbrError::NoTable);
    }
    let mut entries = [PartitionEntry {
        kind: 0,
        first_lba: 0,
        sectors: 0,
    }; 4];
    let mut i = 0;
    while i < 4 {
        let at = ENTRY_BASE + i * ENTRY_LEN;
        let kind = sector0[at + 4];
        if kind == GPT_PROTECTIVE {
            return Err(MbrError::GptProtective);
        }
        entries[i] = PartitionEntry {
            kind,
            first_lba: u32::from_le_bytes([
                sector0[at + 8],
                sector0[at + 9],
                sector0[at + 10],
                sector0[at + 11],
            ]),
            sectors: u32::from_le_bytes([
                sector0[at + 12],
                sector0[at + 13],
                sector0[at + 14],
                sector0[at + 15],
            ]),
        };
        i += 1;
    }
    Ok(entries)
}

/// The store partition's `(first_lba, sectors)`, if the table names one
/// large enough for the [`crate::durable_media`] layout.
pub fn find_store_partition(entries: &[PartitionEntry; 4]) -> Option<(u32, u32)> {
    let mut i = 0;
    while i < 4 {
        let e = entries[i];
        if e.kind == STORE_TYPE
            && e.sectors >= crate::durable_media::MIN_PARTITION_SECTORS
            && e.first_lba != 0
        {
            return Some((e.first_lba, e.sectors));
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sector_with(entries: &[(u8, u32, u32)]) -> [u8; 512] {
        let mut s = [0u8; 512];
        s[510] = 0x55;
        s[511] = 0xAA;
        for (i, &(kind, lba, len)) in entries.iter().enumerate() {
            let at = ENTRY_BASE + i * ENTRY_LEN;
            s[at + 4] = kind;
            s[at + 8..at + 12].copy_from_slice(&lba.to_le_bytes());
            s[at + 12..at + 16].copy_from_slice(&len.to_le_bytes());
        }
        s
    }

    /// The real card this slice deploys to: 512 MiB FAT32 boot at 8192,
    /// 2.5 GiB Linux, then the 1 MiB store partition in the tail.
    #[test]
    fn finds_the_store_on_the_deploy_card_layout() {
        let s = sector_with(&[
            (0x0C, 8192, 1_048_576),
            (0x83, 1_056_768, 5_242_880),
            (STORE_TYPE, 6_299_648, 2048),
        ]);
        let entries = parse(&s).unwrap();
        assert_eq!(find_store_partition(&entries), Some((6_299_648, 2048)));
    }

    #[test]
    fn no_store_entry_is_a_clean_none() {
        let s = sector_with(&[(0x0C, 8192, 1_048_576), (0x83, 1_056_768, 5_242_880)]);
        let entries = parse(&s).unwrap();
        assert_eq!(find_store_partition(&entries), None);
    }

    #[test]
    fn a_hand_shrunk_store_partition_is_refused() {
        let s = sector_with(&[(STORE_TYPE, 8192, 16)]);
        let entries = parse(&s).unwrap();
        assert_eq!(
            find_store_partition(&entries),
            None,
            "smaller than the A/B layout — using it would write past the entry"
        );
    }

    #[test]
    fn gpt_protective_fails_closed() {
        let s = sector_with(&[(GPT_PROTECTIVE, 1, 0xFFFF_FFFF)]);
        assert_eq!(parse(&s), Err(MbrError::GptProtective));
    }

    #[test]
    fn missing_signature_is_no_table() {
        let mut s = sector_with(&[(STORE_TYPE, 8192, 2048)]);
        s[510] = 0;
        assert_eq!(parse(&s), Err(MbrError::NoTable));
    }

    #[test]
    fn zero_lba_store_entry_is_refused() {
        // An entry claiming to start at the MBR itself is table corruption,
        // not a store — writing there would destroy the partition table.
        let s = sector_with(&[(STORE_TYPE, 0, 2048)]);
        let entries = parse(&s).unwrap();
        assert_eq!(find_store_partition(&entries), None);
    }
}
