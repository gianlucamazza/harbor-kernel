//! Kernel owner of the ASID pool (ADR-0050 / K7).
//!
//! Pure free-list arithmetic lives in [`kernel_core::asid`]. This module holds
//! the single pool instance and serialises access with [`IrqSpinLock`]
//! (ADR-0077 — dual-current may destroy/create address spaces concurrently).

use kernel_core::asid::{self, AsidPool};

use crate::sync::{IrqSpinLock, SyncCell};

static POOL: SyncCell<AsidPool> = SyncCell::new(AsidPool::new());
static POOL_LOCK: IrqSpinLock = IrqSpinLock::new();

fn with_pool<R>(f: impl FnOnce(&mut AsidPool) -> R) -> R {
    POOL_LOCK.with(|| {
        // SAFETY: exclusivity from POOL_LOCK (IRQ mask + spin).
        f(unsafe { &mut *POOL.get() })
    })
}

/// Allocate a user ASID, or `None` if the pool is exhausted.
pub fn alloc() -> Option<u16> {
    with_pool(|p| p.alloc())
}

/// Return `asid` to the pool. Returns `true` if it was live (caller must TLBI).
pub fn free(asid: u16) -> bool {
    with_pool(|p| p.free(asid))
}

/// ASIDs still available for user address spaces.
pub fn free_count() -> u16 {
    with_pool(|p| p.free_count())
}

/// Pack a root physical address with `asid` for `TTBR0_EL1`.
#[inline]
pub fn pack_ttbr0(root_phys: usize, asid: u16) -> u64 {
    asid::pack_ttbr0(root_phys as u64, asid)
}
