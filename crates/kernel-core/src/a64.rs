//! AArch64 user-text instruction words (little-endian host encoding helpers).
//!
//! Pure bit patterns for smoke programs and agent payloads. Host-tested so the
//! kernel does not invent SVC/branch encodings by hand in multiple places.

/// `svc #imm` (A64).
#[inline]
pub const fn svc(imm: u16) -> u32 {
    0xD400_0001 | ((imm as u32) << 5)
}

/// `movz xd, #imm16` (LSL #0).
#[inline]
pub const fn movz_x(rd: u8, imm16: u16) -> u32 {
    0xD280_0000 | ((imm16 as u32) << 5) | (rd as u32 & 0x1F)
}

/// `movz xd, #imm16, lsl #16`.
#[inline]
pub const fn movz_x_lsl16(rd: u8, imm16: u16) -> u32 {
    0xD2A0_0000 | ((imm16 as u32) << 5) | (rd as u32 & 0x1F)
}

/// `movk xd, #imm16, lsl #16`.
#[inline]
pub const fn movk_x_lsl16(rd: u8, imm16: u16) -> u32 {
    0xF2A0_0000 | ((imm16 as u32) << 5) | (rd as u32 & 0x1F)
}

/// `movk xd, #imm16, lsl #32`.
#[inline]
pub const fn movk_x_lsl32(rd: u8, imm16: u16) -> u32 {
    0xF2C0_0000 | ((imm16 as u32) << 5) | (rd as u32 & 0x1F)
}

/// `movk xd, #imm16, lsl #48`.
#[inline]
pub const fn movk_x_lsl48(rd: u8, imm16: u16) -> u32 {
    0xF2E0_0000 | ((imm16 as u32) << 5) | (rd as u32 & 0x1F)
}

/// `mov xd, xm` — the register-to-register move, which A64 spells
/// `orr xd, xzr, xm`.
///
/// Needed by an agent that has to move a syscall reply out of the register the
/// ABI delivered it in before making the next call. Assembled by hand here for
/// the same reason as everything else in this module: the alternative is a bare
/// hex constant at the call site.
#[inline]
pub const fn mov_x_reg(rd: u8, rm: u8) -> u32 {
    0xAA00_03E0 | ((rm as u32 & 0x1F) << 16) | (rd as u32 & 0x1F)
}

/// `str xzr, [xn]` — store zero through a register, unscaled offset 0.
///
/// Exists so a deliberately faulting agent can be *written* rather than
/// spelled out in hex at each call site, which is how the M5 probe carried it.
#[inline]
pub const fn str_xzr(rn: u8) -> u32 {
    0xF900_001F | ((rn as u32 & 0x1F) << 5)
}

/// `b .` — branch to self (infinite wait until interrupted or replaced).
#[inline]
pub const fn b_self() -> u32 {
    0x1400_0000
}

/// `b label` — unconditional, `offset_words` signed PC-relative word offset.
/// [`b_self`] is `b_rel(0)`.
#[inline]
pub const fn b_rel(offset_words: i32) -> u32 {
    0x1400_0000 | ((offset_words as u32) & 0x03FF_FFFF)
}

/// `sub xd, xn, #imm12` (64-bit, shift 0).
#[inline]
pub const fn sub_x_imm(rd: u8, rn: u8, imm12: u16) -> u32 {
    0xD100_0000 | ((imm12 as u32 & 0xFFF) << 10) | ((rn as u32 & 0x1F) << 5) | (rd as u32 & 0x1F)
}

/// `cbnz xt, label` — `offset_words` is a signed PC-relative word offset.
#[inline]
pub const fn cbnz_x(rt: u8, offset_words: i32) -> u32 {
    let imm19 = (offset_words as u32) & 0x7_FFFF;
    0xB500_0000 | (imm19 << 5) | (rt as u32 & 0x1F)
}

/// `ldr wt, [xn, #pimm]` — unsigned offset, `pimm` byte offset (multiple of 4).
#[inline]
pub const fn ldr_w_imm(rt: u8, rn: u8, pimm: u16) -> u32 {
    let imm12 = (pimm as u32) / 4;
    0xB940_0000 | (imm12 << 10) | ((rn as u32 & 0x1F) << 5) | (rt as u32 & 0x1F)
}

/// `ldrb wt, [xn]` — zero offset.
#[inline]
pub const fn ldrb_w(rt: u8, rn: u8) -> u32 {
    0x3940_0000 | ((rn as u32 & 0x1F) << 5) | (rt as u32 & 0x1F)
}

/// `tbnz rt, #bit, label` — 32-bit form; `offset_words` signed PC-relative.
#[inline]
pub const fn tbnz_w(rt: u8, bit: u8, offset_words: i32) -> u32 {
    let imm14 = (offset_words as u32) & 0x3FFF;
    0x3700_0000 | ((bit as u32 & 0x1F) << 19) | (imm14 << 5) | (rt as u32 & 0x1F)
}

/// Little-endian bytes of one A64 word.
#[inline]
pub const fn le_bytes(word: u32) -> [u8; 4] {
    word.to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_zero_through_a_register() {
        // Checked against the M5 fault probe, which carried these bytes by hand:
        // `str xzr, [x0]` is 1F 00 00 F9 little-endian.
        assert_eq!(str_xzr(0), 0xF900_001F);
        assert_eq!(le_bytes(str_xzr(0)), [0x1F, 0x00, 0x00, 0xF9]);
        assert_eq!(str_xzr(3), 0xF900_007F);
    }

    #[test]
    fn mov_register_to_register() {
        // Checked against the architectural encoding of `orr Xd, XZR, Xm`:
        // sf=1 opc=01 01010 shift=00 N=0 Rm Rn=11111 Rd.
        assert_eq!(mov_x_reg(0, 2), 0xAA02_03E0); // mov x0, x2
        assert_eq!(mov_x_reg(1, 0), 0xAA00_03E1); // mov x1, x0
        assert_eq!(mov_x_reg(30, 30), 0xAA1E_03FE); // mov x30, x30
    }

    #[test]
    fn svc_encodings() {
        assert_eq!(svc(0), 0xD400_0001);
        assert_eq!(svc(1), 0xD400_0021);
        assert_eq!(svc(2), 0xD400_0041);
    }

    #[test]
    fn movz_x0_ascii() {
        assert_eq!(
            movz_x(0, u16::from(b'H')),
            0xD280_0000 | ((u16::from(b'H') as u32) << 5)
        );
    }

    #[test]
    fn movz_pl011_va_half() {
        // USER_PL011_VA = 0x5000_0000 → movz x0, #0x5000, lsl #16
        assert_eq!(movz_x_lsl16(0, 0x5000), 0xD2A0_0000 | (0x5000 << 5));
    }

    #[test]
    fn movk_shifts() {
        assert_eq!(movk_x_lsl16(2, 0x736e), 0xF2A0_0000 | (0x736e << 5) | 2);
        assert_eq!(movk_x_lsl32(2, 0x6c6f), 0xF2C0_0000 | (0x6c6f << 5) | 2);
        assert_eq!(movk_x_lsl48(2, 0x65), 0xF2E0_0000 | (0x65 << 5) | 2);
    }

    #[test]
    fn ldr_fr_offset() {
        // ldr w1, [x0, #0x18]
        assert_eq!(ldr_w_imm(1, 0, 0x18), 0xB940_1801);
    }

    #[test]
    fn tbnz_rxfe_skip_two_insns() {
        // tbnz w1, #4, +3 words (skip ldrb + svc putc → exit)
        assert_eq!(tbnz_w(1, 4, 3), 0x3700_0000 | (4 << 19) | (3 << 5) | 1);
    }

    #[test]
    fn spin_loop_back_edge() {
        // sub; cbnz x0, 1b → offset is −1 word (not −2: PC is the cbnz itself).
        // Matches gas: `sub x0,x0,#1; cbnz x0, .-4` → 0xb5ffffe0
        assert_eq!(cbnz_x(0, -1), 0xB5FF_FFE0);
    }

    #[test]
    fn spin_program_words_match_gas() {
        // movz x0, #64; sub x0,x0,#1; cbnz x0,1b; svc #1; b .
        assert_eq!(movz_x(0, 64), 0xD280_0800);
        assert_eq!(sub_x_imm(0, 0, 1), 0xD100_0400);
        assert_eq!(cbnz_x(0, -1), 0xB5FF_FFE0);
        assert_eq!(svc(1), 0xD400_0021);
        assert_eq!(b_self(), 0x1400_0000);
    }

    #[test]
    fn b_self_is_infinite() {
        assert_eq!(b_self(), 0x1400_0000);
    }

    #[test]
    fn b_rel_back_edge() {
        // Matches gas: `b .-8` → 0x17fffffe; `b .` is the zero offset.
        assert_eq!(b_rel(-2), 0x17FF_FFFE);
        assert_eq!(b_rel(0), b_self());
        assert_eq!(b_rel(2), 0x1400_0002);
    }
}
