//! BCM2711 / iProc RNG200 register encodings (pure, host-testable).
//!
//! The block produces 32-bit words in a small FIFO. This module owns offsets,
//! bit masks, and packing helpers only — no MMIO. The kernel driver owns the
//! enable / warm-up / read sequence.
//!
//! Hardware output is not a certified entropy source: conditioning and
//! min-entropy are unknown without offline assessment. Callers must not treat
//! these helpers as cryptographic primitives.
//!
//! Compatible string: `brcm,bcm2711-rng200`. Register layout matches the
//! Linux `iproc-rng200` driver and the BCM2711 low-peripheral map.

// --- Offsets (bytes from block base) ---

pub const RNG_CTRL: usize = 0x00;
pub const RNG_SOFT_RESET: usize = 0x04;
pub const RBG_SOFT_RESET: usize = 0x08;
pub const RNG_TOTAL_BIT_COUNT: usize = 0x0C;
pub const RNG_TOTAL_BIT_COUNT_THRESHOLD: usize = 0x10;
pub const RNG_INT_STATUS: usize = 0x18;
pub const RNG_INT_ENABLE: usize = 0x1C;
pub const RNG_FIFO_DATA: usize = 0x20;
pub const RNG_FIFO_COUNT: usize = 0x24;

/// Size of the register window (bytes).
pub const BLOCK_SIZE: usize = 0x28;

// --- RNG_CTRL ---

/// Enable the random bit generator.
pub const CTRL_RBGEN: u32 = 1 << 0;

/// Width of the sample-rate divisor field (bits 13–20).
pub const CTRL_DIV_SHIFT: u32 = 13;
pub const CTRL_DIV_MASK: u32 = 0xFF;

/// Common software sample divisor (~1 MHz class in Linux/community use).
///
/// Not a datasheet guarantee — a conventional encoding shared by existing
/// drivers. Pass `0` to [`ctrl_enable`] for RBGEN-only (Linux `iproc-rng200`
/// init path).
pub const SAMPLE_DIVISOR_DEFAULT: u8 = 0x3;

// --- Soft reset (write 1 to assert; clear to deassert) ---

pub const SOFT_RESET_BIT: u32 = 1 << 0;

// --- RNG_INT_STATUS (R/W1C) ---

/// Master fail / lockout (sticky health failure).
pub const INT_MASTER_FAIL_LOCKOUT: u32 = 1 << 31;
/// NIST continuous-test fail.
pub const INT_NIST_FAIL: u32 = 1 << 5;
/// Startup transitions met (informational).
pub const INT_STARTUP_TRANSITIONS_MET: u32 = 1 << 17;
/// Total bits count threshold crossed (informational).
pub const INT_TOTAL_BITS_COUNT: u32 = 1 << 0;

/// Mask of health failures that require soft-reset recovery.
pub const INT_HEALTH_FAIL_MASK: u32 = INT_MASTER_FAIL_LOCKOUT | INT_NIST_FAIL;

// --- RNG_FIFO_COUNT ---

/// Low 8 bits: number of 32-bit words currently in the FIFO.
pub const FIFO_COUNT_MASK: u32 = 0xFF;

/// Clear-all value written to `RNG_INT_STATUS` after reset.
pub const INT_STATUS_CLEAR_ALL: u32 = 0xFFFF_FFFF;

/// Minimum `RNG_TOTAL_BIT_COUNT` before first read (warm-up floor).
pub const WARMUP_BIT_THRESHOLD: u32 = 16;

/// Build `RNG_CTRL` with `RBGEN` set and optional sample divisor.
///
/// `divisor == 0` leaves bits 13–20 clear (RBGEN only). Non-zero values are
/// placed in bits 13–20.
pub const fn ctrl_enable(divisor: u8) -> u32 {
    let mut ctrl = CTRL_RBGEN;
    if divisor != 0 {
        ctrl |= (divisor as u32) << CTRL_DIV_SHIFT;
    }
    ctrl
}

/// Words available in the FIFO from a `RNG_FIFO_COUNT` register value.
pub const fn fifo_available(fifo_count_reg: u32) -> u8 {
    (fifo_count_reg & FIFO_COUNT_MASK) as u8
}

/// True if `RNG_INT_STATUS` reports lockout or NIST fail.
pub const fn health_failed(int_status: u32) -> bool {
    (int_status & INT_HEALTH_FAIL_MASK) != 0
}

/// True if total bit count has reached the warm-up floor.
pub const fn warmup_done(total_bit_count: u32) -> bool {
    total_bit_count > WARMUP_BIT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_rbgen_only() {
        assert_eq!(ctrl_enable(0), CTRL_RBGEN);
        assert_eq!(ctrl_enable(0) & !CTRL_RBGEN, 0);
    }

    #[test]
    fn ctrl_with_default_divisor() {
        let c = ctrl_enable(SAMPLE_DIVISOR_DEFAULT);
        assert_eq!(c & CTRL_RBGEN, CTRL_RBGEN);
        assert_eq!((c >> CTRL_DIV_SHIFT) & CTRL_DIV_MASK, 0x3);
    }

    #[test]
    fn ctrl_divisor_max_byte() {
        let c = ctrl_enable(0xFF);
        assert_eq!((c >> CTRL_DIV_SHIFT) & CTRL_DIV_MASK, 0xFF);
        assert_eq!(c & CTRL_RBGEN, CTRL_RBGEN);
    }

    #[test]
    fn fifo_available_masks_low_byte() {
        assert_eq!(fifo_available(0), 0);
        assert_eq!(fifo_available(1), 1);
        assert_eq!(fifo_available(0xFF), 255);
        assert_eq!(fifo_available(0xABCD_0010), 0x10);
        assert_eq!(fifo_available(0xFFFF_FF00), 0);
    }

    #[test]
    fn health_failed_on_lockout_or_nist() {
        assert!(!health_failed(0));
        assert!(!health_failed(INT_TOTAL_BITS_COUNT));
        assert!(!health_failed(INT_STARTUP_TRANSITIONS_MET));
        assert!(health_failed(INT_MASTER_FAIL_LOCKOUT));
        assert!(health_failed(INT_NIST_FAIL));
        assert!(health_failed(INT_MASTER_FAIL_LOCKOUT | INT_NIST_FAIL));
        assert!(health_failed(INT_NIST_FAIL | INT_TOTAL_BITS_COUNT));
    }

    #[test]
    fn warmup_threshold() {
        assert!(!warmup_done(0));
        assert!(!warmup_done(WARMUP_BIT_THRESHOLD));
        assert!(warmup_done(WARMUP_BIT_THRESHOLD + 1));
        assert!(warmup_done(u32::MAX));
    }

    #[test]
    fn offsets_are_within_block() {
        for off in [
            RNG_CTRL,
            RNG_SOFT_RESET,
            RBG_SOFT_RESET,
            RNG_TOTAL_BIT_COUNT,
            RNG_TOTAL_BIT_COUNT_THRESHOLD,
            RNG_INT_STATUS,
            RNG_INT_ENABLE,
            RNG_FIFO_DATA,
            RNG_FIFO_COUNT,
        ] {
            assert!(off < BLOCK_SIZE, "offset {off:#x} past block");
            assert_eq!(off % 4, 0, "offset {off:#x} not word-aligned");
        }
    }
}
