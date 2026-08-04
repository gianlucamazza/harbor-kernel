//! AArch64 stage-1 MMU: identity map with 1 GiB blocks (4K granule, 39-bit VA).
//!
//! Which physical ranges are RAM and which are device MMIO is board knowledge:
//! the caller passes them in, so this module never names a peripheral. The bit
//! encodings live in [`kernel_core::paging`] and are unit-tested on the host.

use kernel_core::paging::{self, MemKind, Perms};

use crate::arch::aarch64::cache;
use crate::sync::SyncCell;

/// `TCR_EL1.T0SZ` — 39-bit VA, so the initial lookup level is 1.
const T0SZ: u64 = 25;

/// 512 × 1 GiB entries (level-1 table for 4K granule).
#[repr(C, align(4096))]
struct L1Table {
    entries: [u64; 512],
}

/// The single kernel translation table.
///
/// Written only by [`enable_identity`], with the MMU off and IRQs masked.
static L1: SyncCell<L1Table> = SyncCell::new(L1Table { entries: [0; 512] });

/// Why a requested identity map could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MmuError {
    /// A block base is not 1 GiB aligned or exceeds the 48-bit output address.
    UnmappableBlock(u64),
    /// A block base falls outside the 512 GiB the level-1 table can describe.
    BlockOutOfRange(u64),
}

/// Build identity maps and enable the MMU + I/D caches.
///
/// `ram` and `device` are physical 1 GiB block bases. RAM is mapped writable
/// **and** executable: with 1 GiB blocks there is no way to separate `.text`
/// from the stack and heap, so W^X has to wait for the 4 KiB/2 MiB paging of
/// M3. Device blocks are never executable.
///
/// # Safety
/// Single core; no concurrent page-table writers; IRQs masked. After return,
/// every address the kernel touches must lie in one of the mapped windows.
pub unsafe fn enable_identity(ram: &[u64], device: &[u64]) -> Result<(), MmuError> {
    unsafe {
        // See the doc comment: writable + executable is a known gap, not an oversight.
        const RAM_PERMS: Perms = Perms {
            write: true,
            execute: true,
        };

        let l1 = L1.get();
        let entries = &mut (*l1).entries;
        entries.fill(0);

        for &base in ram {
            entries[index_of(base, entries.len())?] =
                paging::l1_block(base, MemKind::NormalWb, RAM_PERMS)
                    .ok_or(MmuError::UnmappableBlock(base))?;
        }

        for &base in device {
            entries[index_of(base, entries.len())?] =
                paging::l1_block(base, MemKind::Device, Perms::RW)
                    .ok_or(MmuError::UnmappableBlock(base))?;
        }

        let ttbr = l1 as u64;

        core::arch::asm!(
            "msr mair_el1, {v}",
            v = in(reg) paging::mair_el1(),
            options(nostack),
        );
        core::arch::asm!(
            "msr tcr_el1, {v}",
            v = in(reg) paging::tcr_el1_ttbr0_only(T0SZ),
            options(nostack),
        );
        core::arch::asm!(
            "msr ttbr0_el1, {ttbr}",
            "isb",
            ttbr = in(reg) ttbr,
            options(nostack),
        );

        // The table was written with the MMU off, so it went straight to memory —
        // but the walker is about to read it through the caches, and the firmware
        // left lines of its own behind. Invalidate before turning caches on.
        cache::invalidate_dcache_all();
        cache::invalidate_icache();
        cache::invalidate_tlb_all();

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

        Ok(())
    }
}

/// Level-1 index for a 1 GiB block base.
fn index_of(base: u64, entries: usize) -> Result<usize, MmuError> {
    let index = (base / paging::L1_BLOCK_SIZE) as usize;
    if index >= entries {
        return Err(MmuError::BlockOutOfRange(base));
    }
    Ok(index)
}

/// True if `SCTLR_EL1.M` is set.
pub fn is_enabled() -> bool {
    let sctlr: u64;
    // SAFETY: system register read.
    unsafe {
        core::arch::asm!("mrs {v}, sctlr_el1", v = out(reg) sctlr, options(nostack, nomem));
    }
    sctlr & 1 != 0
}
