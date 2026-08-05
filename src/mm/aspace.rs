//! User address-space object (M5 S2) — create/destroy only, no EL0 yet.
//!
//! An [`AddressSpace`] owns a root translation table and every frame allocated
//! for it, all from the ADR-0012 pool ([`super::frames`]). Destroy returns
//! every frame. The root is **not** installed in `TTBR0_EL1` here; S3 will
//! switch TTBR for EL0.

use kernel_core::frame::{FrameId, FrameLedger, LedgerFull};
use kernel_core::paging::{self, ENTRIES_PER_TABLE};

use crate::bsp::board::memmap::FRAME_SIZE;
use crate::mm::frames;

/// Max frames one AS may hold in v1 (root + intermediate tables + user pages).
pub const MAX_AS_FRAMES: usize = 64;

/// Why address-space create failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsError {
    /// Frame pool exhausted or not initialised.
    OutOfFrames,
    /// Internal ledger full (should not hit for a single root).
    LedgerFull,
}

/// Empty user address space: root L1 table only, not live in TTBR0.
pub struct AddressSpace {
    root_phys: usize,
    owned: FrameLedger<MAX_AS_FRAMES>,
}

impl AddressSpace {
    /// Allocate and zero a root table frame; track it for destroy.
    ///
    /// # Errors
    /// [`AsError::OutOfFrames`] if the named pool cannot supply a frame.
    pub fn create() -> Result<Self, AsError> {
        let (root, root_phys) = frames::alloc().ok_or(AsError::OutOfFrames)?;
        // Root must be a zero table before any walk (same contract as kernel arena).
        // SAFETY: phys is identity-mapped RW pool memory exclusive to this frame.
        unsafe {
            let table = root_phys as *mut u64;
            for i in 0..ENTRIES_PER_TABLE {
                core::ptr::write_volatile(table.add(i), 0);
            }
        }

        let mut owned = FrameLedger::new();
        owned
            .push(root.index())
            .map_err(|LedgerFull| AsError::LedgerFull)?;

        // Sanity: table size matches frame granule.
        debug_assert_eq!(FRAME_SIZE as u64, paging::PAGE_SIZE);
        debug_assert_eq!(
            core::mem::size_of::<[u64; ENTRIES_PER_TABLE]>(),
            FRAME_SIZE
        );

        let _ = root; // ownership tracked only via ledger + root_phys
        Ok(Self { root_phys, owned })
    }

    /// Physical address of the root table (future `TTBR0_EL1` value).
    #[inline]
    pub fn root_phys(&self) -> usize {
        self.root_phys
    }

    /// How many pool frames this AS currently owns.
    #[inline]
    pub fn frame_count(&self) -> usize {
        self.owned.len()
    }

    /// Record an additional frame allocated for this AS (maps in S3+).
    ///
    /// Caller must have obtained `id` from [`frames::alloc`].
    #[allow(dead_code)] // S3 map helpers
    pub fn track(&mut self, id: FrameId) -> Result<(), AsError> {
        self.owned
            .push(id.index())
            .map_err(|LedgerFull| AsError::LedgerFull)
    }

    /// Free every owned frame back to the pool. Consumes the AS.
    pub fn destroy(mut self) {
        for &index in self.owned.as_slice() {
            let id = FrameId::from_index(index);
            // Best-effort: double-free would mean a bug in track/create.
            let _ = frames::free(id);
        }
        self.owned.clear();
        // Root is included in `owned`; forget self without Drop double-free.
        core::mem::forget(self);
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        // If destroy was not called, still return frames (leak-proof).
        for &index in self.owned.as_slice() {
            let _ = frames::free(FrameId::from_index(index));
        }
        self.owned.clear();
    }
}
