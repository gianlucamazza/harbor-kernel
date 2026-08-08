//! Kernel owner of the ASID pool (ADR-0050 / K7).
//!
//! Pure free-list arithmetic lives in [`kernel_core::asid`]. This module holds
//! the single pool instance and serialises access with IRQ masking, matching
//! the frame pool pattern.

use kernel_core::asid::{self, AsidPool};

use crate::arch::cpu;
use crate::sync::SyncCell;

static POOL: SyncCell<AsidPool> = SyncCell::new(AsidPool::new());

/// Allocate a user ASID, or `None` if the pool is exhausted.
pub fn alloc() -> Option<u16> {
    cpu::without_irqs(|| {
        // SAFETY: IRQ-masked exclusive access to the sole pool.
        unsafe { (*POOL.get()).alloc() }
    })
}

/// Return `asid` to the pool. Returns `true` if it was live (caller must TLBI).
pub fn free(asid: u16) -> bool {
    cpu::without_irqs(|| {
        // SAFETY: IRQ-masked exclusive access to the sole pool.
        unsafe { (*POOL.get()).free(asid) }
    })
}

/// ASIDs still available for user address spaces.
pub fn free_count() -> u16 {
    cpu::without_irqs(|| {
        // SAFETY: IRQ-masked shared read of the sole pool.
        unsafe { (*POOL.get()).free_count() }
    })
}

/// Pack a root physical address with `asid` for `TTBR0_EL1`.
#[inline]
pub fn pack_ttbr0(root_phys: usize, asid: u16) -> u64 {
    asid::pack_ttbr0(root_phys as u64, asid)
}
