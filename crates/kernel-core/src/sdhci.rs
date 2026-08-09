//! SDHCI host-controller register encodings (pure, host-testable).
//!
//! The BCM2711 EMMC2 block is a standard SDHCI host. This module owns byte
//! offsets, bitfields, the command-register encoder and the divided-clock
//! arithmetic — no MMIO. The kernel driver owns reset, clock bring-up and
//! the command/data sequencing; the card init *policy* lives in
//! [`crate::sdcard`].
//!
//! Register names follow the SDHCI spec as the BCM2711 datasheet renders
//! them (`BLKSIZECNT`, `CMDTM`, …).

// --- Offsets (bytes from block base) ---

pub const ARG2: usize = 0x00;
pub const BLKSIZECNT: usize = 0x04;
pub const ARG1: usize = 0x08;
pub const CMDTM: usize = 0x0C;
pub const RESP0: usize = 0x10;
pub const RESP1: usize = 0x14;
pub const RESP2: usize = 0x18;
pub const RESP3: usize = 0x1C;
pub const DATA: usize = 0x20;
pub const STATUS: usize = 0x24;
pub const CONTROL0: usize = 0x28;
pub const CONTROL1: usize = 0x2C;
pub const INTERRUPT: usize = 0x30;
pub const IRPT_MASK: usize = 0x34;
pub const IRPT_EN: usize = 0x38;
pub const CONTROL2: usize = 0x3C;
pub const SLOTISR_VER: usize = 0xFC;

/// Size of the register window (bytes).
pub const BLOCK_SIZE: usize = 0x100;

/// One SD sector; the only block size this slice ever programs.
pub const SECTOR_SIZE: u32 = 512;

// --- STATUS ---

pub const STATUS_CMD_INHIBIT: u32 = 1 << 0;
pub const STATUS_DAT_INHIBIT: u32 = 1 << 1;

// --- CONTROL0 ---

/// SD bus power on (Power Control, byte 1 of the CONTROL0 word).
pub const C0_BUS_POWER: u32 = 1 << 8;
/// Bus voltage select 3.3 V.
pub const C0_VOLTAGE_3V3: u32 = 0b111 << 9;

// --- CONTROL1 ---

pub const C1_CLK_INTLEN: u32 = 1 << 0;
/// Read-only: internal clock stable.
pub const C1_CLK_STABLE: u32 = 1 << 1;
pub const C1_CLK_EN: u32 = 1 << 2;
/// Data timeout unit exponent field (TMCLK * 2^(13+x)); 0b1110 is the
/// largest defined value and the conservative choice for PIO.
pub const C1_TOUNIT_MAX: u32 = 0b1110 << 16;
pub const C1_SRST_HC: u32 = 1 << 24;
pub const C1_SRST_CMD: u32 = 1 << 25;
pub const C1_SRST_DATA: u32 = 1 << 26;

/// SDHCI v3 10-bit divided-clock: low 8 divisor bits go in [15:8], the top
/// 2 bits in [7:6].
#[inline]
pub const fn c1_clock_bits(divisor10: u16) -> u32 {
    let d = divisor10 as u32 & 0x3FF;
    ((d & 0xFF) << 8) | ((d >> 8) << 6)
}

/// Smallest 10-bit divisor giving `base_hz / (2*div) <= target_hz`.
///
/// Divisor 0 means "base clock through"; requesting a target at or above
/// the base returns 0. A target too slow for the 10-bit field saturates at
/// 0x3FF — the caller gets the slowest clock the host can make, which for
/// the 400 kHz init phase on any plausible base clock is still in spec.
#[inline]
pub const fn divider_for(base_hz: u32, target_hz: u32) -> u16 {
    if target_hz == 0 {
        return 0x3FF;
    }
    if target_hz >= base_hz {
        return 0;
    }
    // ceil(base / (2*target)), then saturate to the field width.
    let div = base_hz.div_ceil(2 * target_hz);
    if div > 0x3FF { 0x3FF } else { div as u16 }
}

/// The divided clock a 10-bit divisor actually produces.
#[inline]
pub const fn divided_clock_hz(base_hz: u32, divisor10: u16) -> u32 {
    if divisor10 == 0 {
        base_hz
    } else {
        base_hz / (2 * divisor10 as u32)
    }
}

// --- CMDTM ---

/// Response type field ([17:16]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RespType {
    None,
    /// 136-bit (R2: CID/CSD).
    Long,
    /// 48-bit (R1/R3/R6/R7).
    Short,
    /// 48-bit with busy (R1b).
    ShortBusy,
}

/// Data direction for a data command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataDir {
    /// No data phase.
    None,
    /// Card → host (read).
    Read,
    /// Host → card (write).
    Write,
}

const TM_DAT_DIR_READ: u32 = 1 << 4;
const TM_ISDATA: u32 = 1 << 21;
const TM_CRCCHK_EN: u32 = 1 << 19;
const TM_IXCHK_EN: u32 = 1 << 20;

/// Encode the `CMDTM` word for one command.
///
/// CRC and index checks follow the response type: a 48-bit response carries
/// both; R2 (long) has a CRC but no index; R3 (`OpCond`, encoded by its
/// caller as [`RespType::Short`] **without** checks via
/// [`cmdtm_no_checks`]) protects neither.
#[inline]
pub const fn cmdtm(index: u8, resp: RespType, dir: DataDir) -> u32 {
    let mut w = (index as u32 & 0x3F) << 24;
    w |= match resp {
        RespType::None => 0,
        RespType::Long => (0b01 << 16) | TM_CRCCHK_EN,
        RespType::Short => (0b10 << 16) | TM_CRCCHK_EN | TM_IXCHK_EN,
        RespType::ShortBusy => (0b11 << 16) | TM_CRCCHK_EN | TM_IXCHK_EN,
    };
    match dir {
        DataDir::None => {}
        DataDir::Read => w |= TM_ISDATA | TM_DAT_DIR_READ,
        DataDir::Write => w |= TM_ISDATA,
    }
    w
}

/// [`cmdtm`] with CRC/index checks stripped — R3 (ACMD41) has no CRC and
/// its response index field is reserved, so checking either fails good
/// cards.
#[inline]
pub const fn cmdtm_no_checks(index: u8, resp: RespType, dir: DataDir) -> u32 {
    cmdtm(index, resp, dir) & !(TM_CRCCHK_EN | TM_IXCHK_EN)
}

// --- INTERRUPT (status; W1C) ---

pub const INT_CMD_DONE: u32 = 1 << 0;
pub const INT_DATA_DONE: u32 = 1 << 1;
pub const INT_WRITE_RDY: u32 = 1 << 4;
pub const INT_READ_RDY: u32 = 1 << 5;
/// Any-error summary bit.
pub const INT_ERR: u32 = 1 << 15;
pub const INT_CTO_ERR: u32 = 1 << 16;
pub const INT_CCRC_ERR: u32 = 1 << 17;
pub const INT_DTO_ERR: u32 = 1 << 20;
pub const INT_DCRC_ERR: u32 = 1 << 21;

/// Everything above bit 15 plus the summary bit: what a bounded poll treats
/// as "this command failed".
pub const INT_ERROR_MASK: u32 = 0xFFFF_0000 | INT_ERR;

/// True when `interrupt` reports a command timeout and nothing else fatal —
/// the one error init deliberately interprets (CMD8 on a legacy card).
#[inline]
pub const fn is_timeout_only(interrupt: u32) -> bool {
    interrupt & INT_ERROR_MASK != 0 && interrupt & INT_ERROR_MASK & !(INT_ERR | INT_CTO_ERR) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divider_math_edges() {
        // 200 MHz base → 400 kHz init: ceil(200e6/800e3) = 250.
        assert_eq!(divider_for(200_000_000, 400_000), 250);
        assert_eq!(divided_clock_hz(200_000_000, 250), 400_000);
        // 200 MHz → 25 MHz data: exact division, 4.
        assert_eq!(divider_for(200_000_000, 25_000_000), 4);
        assert_eq!(divided_clock_hz(200_000_000, 4), 25_000_000);
        // A 100 MHz real base with the 200 MHz-sized divisors stays in spec.
        assert!(divided_clock_hz(100_000_000, 250) <= 400_000);
        assert!(divided_clock_hz(100_000_000, 4) <= 25_000_000);
        // Target at/above base → divisor 0 (pass-through).
        assert_eq!(divider_for(50_000_000, 50_000_000), 0);
        // Absurdly slow target saturates the field.
        assert_eq!(divider_for(200_000_000, 1), 0x3FF);
        assert_eq!(divider_for(200_000_000, 0), 0x3FF);
    }

    #[test]
    fn clock_bits_split_the_ten_bit_divisor() {
        // 250 = 0b00_1111_1010: low byte 0xFA in [15:8], top bits 00.
        assert_eq!(c1_clock_bits(250), 0xFA << 8);
        // 0x3FF: low byte 0xFF in [15:8], top 0b11 in [7:6].
        assert_eq!(c1_clock_bits(0x3FF), (0xFF << 8) | (0b11 << 6));
    }

    #[test]
    fn cmdtm_encodings() {
        // CMD17 READ_SINGLE_BLOCK: index 17, R1, read data.
        let w = cmdtm(17, RespType::Short, DataDir::Read);
        assert_eq!(w >> 24, 17);
        assert_eq!((w >> 16) & 0b11, 0b10);
        assert_ne!(w & TM_ISDATA, 0);
        assert_ne!(w & TM_DAT_DIR_READ, 0);
        assert_ne!(w & TM_CRCCHK_EN, 0);
        // CMD24 WRITE_BLOCK: write direction bit clear.
        let w = cmdtm(24, RespType::Short, DataDir::Write);
        assert_ne!(w & TM_ISDATA, 0);
        assert_eq!(w & TM_DAT_DIR_READ, 0);
        // CMD2 ALL_SEND_CID: long response, CRC but no index check.
        let w = cmdtm(2, RespType::Long, DataDir::None);
        assert_eq!((w >> 16) & 0b11, 0b01);
        assert_ne!(w & TM_CRCCHK_EN, 0);
        assert_eq!(w & TM_IXCHK_EN, 0);
        // ACMD41: R3 carries no CRC and a reserved index field.
        let w = cmdtm_no_checks(41, RespType::Short, DataDir::None);
        assert_eq!(w & (TM_CRCCHK_EN | TM_IXCHK_EN), 0);
    }

    #[test]
    fn timeout_only_is_discriminated_from_real_errors() {
        assert!(is_timeout_only(INT_ERR | INT_CTO_ERR));
        assert!(!is_timeout_only(INT_ERR | INT_CCRC_ERR));
        assert!(!is_timeout_only(INT_ERR | INT_CTO_ERR | INT_DCRC_ERR));
        assert!(!is_timeout_only(INT_CMD_DONE));
    }
}
