//! User address space (M5 S2–S3) — ADR-0012 frames + ADR-0014 TTBR0 regime.
//!
//! Create allocates a root; [`AddressSpace::prepare_for_el0`] deep-clones kernel
//! coverage into it and maps the private user window. Destroy frees every
//! tracked frame. EL0 entry is [`crate::arch::el0`].

// Audit debt (2026-08-06): 4 unsafe blocks here predate
// `clippy::undocumented_unsafe_blocks` and do not yet say what makes them sound.
// This comes off when the audit reaches this module and the SAFETY comments can
// state something checkable rather than restate the code. See Cargo.toml.
#![allow(clippy::undocumented_unsafe_blocks)]

use kernel_core::frame::{FrameId, FrameLedger, LedgerFull};
use kernel_core::paging::{
    self, ENTRIES_PER_TABLE, Level, MemKind, PAGE_SIZE, Perms, table_descriptor,
};

use crate::arch::{cache, mmu};
use crate::bsp::board::memmap::{FRAME_SIZE, USER_STACK_PAGES, USER_STACK_TOP, USER_VA_BASE};
use crate::mm::frames;

/// Max frames one AS may hold (root + cloned tables + user stack pages).
pub const MAX_AS_FRAMES: usize = 256;

/// Why address-space create / prepare failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsError {
    /// Frame pool exhausted or not initialised.
    OutOfFrames,
    /// Internal ledger full.
    LedgerFull,
    /// Kernel map not activated yet.
    NoKernelRoot,
    /// Clone / map walked an unexpected descriptor.
    BadTable,
    /// Already prepared for EL0.
    AlreadyPrepared,
    /// VA/PA/len not page-aligned or zero.
    Unaligned,
    /// Write would leave the frame it was validated against.
    OutOfRange,
}

/// User address space: own TTBR0 root, not necessarily live.
pub struct AddressSpace {
    root_phys: usize,
    owned: FrameLedger<MAX_AS_FRAMES>,
    /// User stack top VA (initial SP_EL0) after prepare; 0 if not prepared.
    user_sp: u64,
    /// Phys of the lowest user stack page (code/data poke for probes).
    user_base_phys: usize,
    prepared: bool,
}

impl AddressSpace {
    /// Allocate and zero a root L1 table frame.
    pub fn create() -> Result<Self, AsError> {
        let (root, root_phys) = frames::alloc().ok_or(AsError::OutOfFrames)?;
        // SAFETY: identity-mapped pool frame exclusive to us.
        unsafe {
            zero_table(root_phys);
        }

        let mut owned = FrameLedger::new();
        owned
            .push(root.index())
            .map_err(|LedgerFull| AsError::LedgerFull)?;

        debug_assert_eq!(FRAME_SIZE as u64, PAGE_SIZE);

        let _ = root;
        Ok(Self {
            root_phys,
            owned,
            user_sp: 0,
            user_base_phys: 0,
            prepared: false,
        })
    }

    /// Deep-clone live kernel coverage into this root and map the user stack.
    ///
    /// ADR-0014: leaf descriptors for kernel memory are copied (shared PA);
    /// intermediate tables are new frames from the user pool.
    pub fn prepare_for_el0(&mut self) -> Result<(), AsError> {
        if self.prepared {
            return Err(AsError::AlreadyPrepared);
        }
        let kroot = mmu::kernel_root_phys().ok_or(AsError::NoKernelRoot)?;
        // SAFETY: kernel root is live identity-mapped table memory.
        unsafe {
            self.clone_table_into(kroot as *const u64, self.root_phys as *mut u64, Level::L1)?;
        }
        self.map_user_stack()?;
        self.prepared = true;
        Ok(())
    }

    /// Physical root for `TTBR0_EL1`.
    #[inline]
    pub fn root_phys(&self) -> usize {
        self.root_phys
    }

    /// Initial user SP (stack top) after prepare; 0 if not prepared.
    #[inline]
    pub fn user_sp(&self) -> u64 {
        self.user_sp
    }

    /// User entry VA (bottom of stack window) after prepare.
    #[inline]
    pub fn user_entry_va(&self) -> u64 {
        if self.prepared { USER_VA_BASE } else { 0 }
    }

    /// Map one page of **device** MMIO into this AS (ADR-0013 agent windows).
    ///
    /// `va` and `pa` must be page-aligned. Does not allocate a RAM frame for
    /// the leaf — only intermediate page-table frames. Leaves are
    /// Device-nGnRnE with the given EL0-capable `perms` (typically
    /// [`Perms::USER_RW`] for a UART agent).
    ///
    /// Call after [`prepare_for_el0`] so the user root already holds kernel
    /// coverage; `va` must not collide with an existing leaf.
    pub fn map_device_page(&mut self, va: u64, pa: u64, perms: Perms) -> Result<(), AsError> {
        if !self.prepared {
            return Err(AsError::BadTable);
        }
        if !va.is_multiple_of(PAGE_SIZE) || !pa.is_multiple_of(PAGE_SIZE) {
            return Err(AsError::Unaligned);
        }
        // SAFETY: exclusive AS tables; pa is a named BSP device page.
        unsafe { self.map_l3_page(va, pa, MemKind::Device, perms) }
    }

    /// Write raw bytes into the **first** page of the user window (kernel
    /// identity access to phys).
    ///
    /// After the store, publishes the range for instruction fetch (D clean to
    /// PoU + I invalidate). Required on Cortex-A72 whenever the bytes may run
    /// at EL0 — not optional for QEMU.
    ///
    /// The bound is one frame, not the whole window: `user_base_phys` is the
    /// physical address of page 0 alone, and [`Self::map_user_stack`] takes the
    /// remaining pages from separate [`frames::alloc`] calls that are contiguous
    /// only by accident of the pool's free order. Validating against the window
    /// would license a write that lands in whatever frame follows page 0 —
    /// another live address space's tables, after any create/destroy cycle.
    pub fn poke_user(&self, offset: usize, bytes: &[u8]) -> Result<(), AsError> {
        if !self.prepared || self.user_base_phys == 0 {
            return Err(AsError::BadTable);
        }
        if offset.saturating_add(bytes.len()) > FRAME_SIZE {
            return Err(AsError::OutOfRange);
        }
        let dest = self.user_base_phys + offset;
        // SAFETY: prepared pages are pool frames, identity-mapped RW for EL1.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dest as *mut u8, bytes.len());
            cache::publish_executable(dest, bytes.len());
        }
        Ok(())
    }

    #[inline]
    pub fn frame_count(&self) -> usize {
        self.owned.len()
    }

    /// Record an additional owned frame.
    pub fn track(&mut self, id: FrameId) -> Result<(), AsError> {
        self.owned
            .push(id.index())
            .map_err(|LedgerFull| AsError::LedgerFull)
    }

    /// Free every owned frame. Consumes the AS.
    pub fn destroy(mut self) {
        for &index in self.owned.as_slice() {
            let _ = frames::free(FrameId::from_index(index));
        }
        self.owned.clear();
        core::mem::forget(self);
    }

    /// Map the private user window at [`USER_VA_BASE`].
    ///
    /// Layout (M5 v1, fixed in BSP): page 0 is user text (`USER_RX`); pages
    /// 1..n-1 are stack (`USER_RW`); `SP_EL0` starts at [`USER_STACK_TOP`].
    /// Kernel leaves share PA and keep EL0-denied AP from the clone step.
    fn map_user_stack(&mut self) -> Result<(), AsError> {
        let mut va = USER_VA_BASE;
        let mut first_phys = 0usize;
        for i in 0..USER_STACK_PAGES {
            let (id, phys) = frames::alloc().ok_or(AsError::OutOfFrames)?;
            self.track(id)?;
            if i == 0 {
                first_phys = phys;
            }
            // SAFETY: exclusive new frame.
            unsafe {
                core::ptr::write_bytes(phys as *mut u8, 0, FRAME_SIZE);
            }
            let perms = if i == 0 {
                Perms::USER_RX
            } else {
                Perms::USER_RW
            };
            unsafe {
                self.map_l3_page(va, phys as u64, MemKind::NormalWb, perms)?;
            }
            va += PAGE_SIZE;
        }
        self.user_base_phys = first_phys;
        self.user_sp = USER_STACK_TOP;
        Ok(())
    }

    /// Install one L3 page mapping at `va` → `pa` under this AS root.
    unsafe fn map_l3_page(
        &mut self,
        va: u64,
        pa: u64,
        kind: MemKind,
        perms: Perms,
    ) -> Result<(), AsError> {
        unsafe {
            let mut table = self.root_phys as *mut u64;
            let mut level = Level::L1;
            while level != Level::L3 {
                let index = level.index(va);
                let entry = core::ptr::read_volatile(table.add(index));
                let next_phys = if paging::is_invalid(entry) {
                    let (id, phys) = frames::alloc().ok_or(AsError::OutOfFrames)?;
                    self.track(id)?;
                    zero_table(phys);
                    let desc = table_descriptor(phys as u64).ok_or(AsError::BadTable)?;
                    core::ptr::write_volatile(table.add(index), desc);
                    phys as u64
                } else if paging::is_table(entry, level) {
                    paging::descriptor_address(entry)
                } else {
                    return Err(AsError::BadTable);
                };
                table = next_phys as *mut u64;
                level = level.next().ok_or(AsError::BadTable)?;
            }
            let index = Level::L3.index(va);
            if !paging::is_invalid(core::ptr::read_volatile(table.add(index))) {
                return Err(AsError::BadTable);
            }
            let leaf = paging::leaf(Level::L3, pa, kind, perms).ok_or(AsError::BadTable)?;
            core::ptr::write_volatile(table.add(index), leaf);
            Ok(())
        }
    }

    /// Clone `src` table at `level` into pre-allocated zeroed `dst` table memory.
    unsafe fn clone_table_into(
        &mut self,
        src: *const u64,
        dst: *mut u64,
        level: Level,
    ) -> Result<(), AsError> {
        unsafe {
            for i in 0..ENTRIES_PER_TABLE {
                let e = core::ptr::read_volatile(src.add(i));
                if paging::is_invalid(e) {
                    core::ptr::write_volatile(dst.add(i), 0);
                    continue;
                }
                if paging::is_leaf(e, level) {
                    // Share physical data page / block; AP already EL0-denied for kernel.
                    core::ptr::write_volatile(dst.add(i), e);
                    continue;
                }
                if paging::is_table(e, level) {
                    let child_src = paging::descriptor_address(e) as *const u64;
                    let (id, child_phys) = frames::alloc().ok_or(AsError::OutOfFrames)?;
                    self.track(id)?;
                    zero_table(child_phys);
                    let next = level.next().ok_or(AsError::BadTable)?;
                    self.clone_table_into(child_src, child_phys as *mut u64, next)?;
                    let desc = table_descriptor(child_phys as u64).ok_or(AsError::BadTable)?;
                    core::ptr::write_volatile(dst.add(i), desc);
                    continue;
                }
                return Err(AsError::BadTable);
            }
            Ok(())
        }
    }
}

impl Drop for AddressSpace {
    fn drop(&mut self) {
        for &index in self.owned.as_slice() {
            let _ = frames::free(FrameId::from_index(index));
        }
        self.owned.clear();
    }
}

unsafe fn zero_table(phys: usize) {
    unsafe {
        let table = phys as *mut u64;
        for i in 0..ENTRIES_PER_TABLE {
            core::ptr::write_volatile(table.add(i), 0);
        }
    }
}
