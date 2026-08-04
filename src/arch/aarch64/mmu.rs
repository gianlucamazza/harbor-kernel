//! AArch64 stage-1 MMU: identity map with 1 GiB blocks (4K granule, 39-bit VA).
//!
//! Layout (T0SZ=25 → initial lookup level 1):
//! - L1[0], L1[1]: Normal WB RAM (`0x0000_0000`–`0x7FFF_FFFF`)
//! - L1[3]: Device-nGnRnE (`0xC000_0000`–`0xFFFF_FFFF`) — UART + GIC

use core::ptr::addr_of_mut;

/// 512 × 1 GiB entries (level-1 table for 4K granule).
#[repr(C, align(4096))]
struct L1Table {
    entries: [u64; 512],
}

static mut L1: L1Table = L1Table { entries: [0; 512] };

// Stage-1 block descriptor bits (4K granule).
const DESC_BLOCK: u64 = 0b01;
const DESC_AF: u64 = 1 << 10;
const DESC_SH_IS: u64 = 0b11 << 8; // Inner Shareable
const DESC_AP_EL1_RW: u64 = 0b00 << 6;
const DESC_UXN: u64 = 1 << 54;
const DESC_PXN: u64 = 1 << 53;

const ATTR_NORMAL: u64 = 0 << 2; // MAIR Attr0
const ATTR_DEVICE: u64 = 1 << 2; // MAIR Attr1

const MAIR_NORMAL_WB: u64 = 0xFF;
const MAIR_DEVICE_NGNRNE: u64 = 0x00;

/// End of identity-mapped low RAM (2 × 1 GiB blocks).
pub const IDENTITY_RAM_END: usize = 0x8000_0000;

/// Build identity maps and enable the MMU + I/D caches.
///
/// # Safety
/// Single core; no concurrent page-table writers. After return, kernel RAM and
/// device MMIO used by this project must lie in the mapped windows.
pub unsafe fn enable_identity() {
    let l1 = addr_of_mut!(L1);
    let entries = &mut (*l1).entries;

    for e in entries.iter_mut() {
        *e = 0;
    }

    // Kernel + heap live in low RAM.
    entries[0] = block_ram(0x0000_0000);
    entries[1] = block_ram(0x4000_0000);

    // Peripherals at 0xFE00_0000 and GIC at 0xFF84_0000 sit in this 1 GiB block.
    entries[3] = block_device(0xC000_0000);

    let ttbr = l1 as u64;

    let mair = MAIR_NORMAL_WB | (MAIR_DEVICE_NGNRNE << 8);
    core::arch::asm!("msr mair_el1, {v}", v = in(reg) mair, options(nostack));

    // T0SZ=25 (39-bit VA), TG0=4K, inner shareable, WB WA, IPS=40-bit.
    // TG0=4K is 0 in [15:14]. IPS=40-bit in [34:32].
    let tcr: u64 =
        25 | (0b01 << 8) | (0b01 << 10) | (0b11 << 12) | (0b010u64 << 32);
    core::arch::asm!("msr tcr_el1, {v}", v = in(reg) tcr, options(nostack));

    core::arch::asm!(
        "msr ttbr0_el1, {ttbr}",
        "isb",
        ttbr = in(reg) ttbr,
        options(nostack),
    );

    core::arch::asm!(
        "dsb ish",
        "tlbi vmalle1",
        "dsb ish",
        "isb",
        options(nostack),
    );

    let mut sctlr: u64;
    core::arch::asm!("mrs {v}, sctlr_el1", v = out(reg) sctlr, options(nostack));
    sctlr |= (1 << 0) | (1 << 2) | (1 << 12); // M | C | I
    sctlr &= !(1 << 1); // clear strict alignment bit
    core::arch::asm!(
        "msr sctlr_el1, {v}",
        "isb",
        v = in(reg) sctlr,
        options(nostack),
    );
}

fn block_ram(pa: u64) -> u64 {
    // Executable at EL1 (no PXN), not at EL0 (UXN).
    (pa & 0x0000_FFFF_C000_0000)
        | ATTR_NORMAL
        | DESC_AP_EL1_RW
        | DESC_SH_IS
        | DESC_AF
        | DESC_UXN
        | DESC_BLOCK
}

fn block_device(pa: u64) -> u64 {
    (pa & 0x0000_FFFF_C000_0000)
        | ATTR_DEVICE
        | DESC_AP_EL1_RW
        | DESC_SH_IS
        | DESC_AF
        | DESC_UXN
        | DESC_PXN
        | DESC_BLOCK
}

/// True if SCTLR_EL1.M is set.
pub fn is_enabled() -> bool {
    let sctlr: u64;
    // SAFETY: system register read.
    unsafe {
        core::arch::asm!("mrs {v}, sctlr_el1", v = out(reg) sctlr, options(nostack, nomem));
    }
    sctlr & 1 != 0
}
