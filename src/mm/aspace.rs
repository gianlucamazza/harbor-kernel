//! User address space (M5 S2–S3) — ADR-0012 frames + ADR-0014 TTBR0 regime.
//!
//! Create allocates a root; [`AddressSpace::prepare_for_el0`] deep-clones kernel
//! coverage into it and maps the private user window. Destroy frees every
//! tracked frame. EL0 entry is [`crate::arch::el0`].

use kernel_core::frame::{FrameId, FrameLedger, LedgerFull};
use kernel_core::paging::{
    self, ENTRIES_PER_TABLE, Level, MemKind, PAGE_SIZE, Perms, table_descriptor,
};

use crate::arch::{cache, mmu};
use crate::bsp::board::memmap::{FRAME_SIZE, USER_STACK_PAGES, USER_VA_BASE};
use crate::mm::frames;
use kernel_core::layout::UserWindow;

/// The window an [`AddressSpace::create`] gets when nobody asks for another.
///
/// One page of text and the rest stack, which is what every agent had before a
/// manifest could ask for more (ADR-0021 §5). Geometry and bounds live in
/// [`UserWindow`], where they are host-tested; this only names the board's
/// numbers.
const DEFAULT_WINDOW: UserWindow = UserWindow {
    base: USER_VA_BASE,
    pages: USER_STACK_PAGES,
    text_pages: 1,
    frame: FRAME_SIZE,
};

/// Most executable pages one agent may declare: 64 KiB of text.
///
/// A ceiling rather than a target. It bounds [`AddressSpace::text_phys`], which
/// is an array because the frames behind the text are **not contiguous** and the
/// kernel has to know each one's physical address to write it. Against a 512
/// frame pool, an agent at this ceiling costs an eighth of it — refused as an
/// error, never a panic.
pub const MAX_TEXT_PAGES: usize = 16;

// ## Three things that are correct today for reasons written somewhere else
//
// The audit of 2026-08-06 checked each of these and found no bug. They are
// recorded because in every case the reason lives in another file, and the day
// one of them changes there is nothing here that would notice.
//
// **1. A cloned address space diverges from the kernel map.**
// `prepare_for_el0` deep-copies kernel coverage into the user root, and
// `mmu::map` / `mmu::unmap` afterwards mutate only the kernel root. Any spawn
// or exit while a prepared AS is alive leaves that AS with a stale view — a
// task-stack guard page unmapped after the clone is still mapped here.
//
// It is not reachable because of what runs under the user root: only EL0 code,
// which every kernel leaf denies, and the few instructions of `kernel_entry` in
// `vectors.s` between a lower-EL exception and `switch_ttbr0`, which touch the
// exception stack and nothing else. The exception stack is a `link.ld` region
// mapped once by `activate` and never re-mapped, so it cannot be the leaf that
// drifted. It becomes reachable the moment kernel code runs under a user root
// and touches the heap.
//
// **2. `destroy` frees frames and invalidates by ASID.**
// With K7, `switch_ttbr0` no longer always does `tlbi vmalle1is`: ASID-tagged
// user leaves stay in the TLB across switches. So `destroy` must free the ASID
// and run `tlbi aside1is` for that tag before another AS can reuse it. An AS
// that never entered EL0 has nothing cached; the ASID invalidate is then a
// no-op in practice but still the correct contract.
//
// **3. The user window's pages are contiguous only by accident.**
// `map_user_window` takes a separate frame per page from a pool whose free list
// is LIFO, so a fresh boot hands out consecutive ones. `poke_user` used to
// validate against the whole window while writing from page 0's physical
// address, which turned that accident into the bound.
//
// Multi-page text (ADR-0021) could have re-introduced exactly that bug in a form
// that works on a fresh boot and corrupts another address space after the first
// create/destroy cycle. It does not, because nothing assumes adjacency: the
// physical address of every text page is recorded at map time and `poke_user`
// walks them one at a time.

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
    /// ASID pool exhausted (more concurrent ASes than 8-bit pool).
    OutOfAsid,
}

/// User address space: own TTBR0 root, not necessarily live.
pub struct AddressSpace {
    root_phys: usize,
    /// ASID assigned at create (non-zero); freed on destroy.
    asid: u16,
    owned: FrameLedger<MAX_AS_FRAMES>,
    /// User stack top VA (initial SP_EL0) after prepare; 0 if not prepared.
    user_sp: u64,
    /// Physical address of each text page, in window order.
    ///
    /// An array and not a base+length, because these frames are separate
    /// [`frames::alloc`] results and adjacent only by luck. Entries at and above
    /// `window.text_pages` are zero.
    text_phys: [usize; MAX_TEXT_PAGES],
    /// This AS's own geometry, from the manifest entry that asked for it.
    window: UserWindow,
    prepared: bool,
}

impl AddressSpace {
    /// Allocate and zero a root L1 table frame, with the default window.
    pub fn create() -> Result<Self, AsError> {
        Self::create_with(
            DEFAULT_WINDOW.text_pages,
            DEFAULT_WINDOW.pages - DEFAULT_WINDOW.text_pages,
        )
    }

    /// Allocate a root for an agent that declared its own geometry.
    ///
    /// The refusals happen here, before a single frame is taken: a window with
    /// no text or no stack, and text past [`MAX_TEXT_PAGES`]. An agent asking
    /// for more than the pool can spare is an error the loader reports, not a
    /// panic — ADR-0021's frame budget is a consequence, not an assumption.
    pub fn create_with(text_pages: usize, stack_pages: usize) -> Result<Self, AsError> {
        let window = UserWindow {
            base: USER_VA_BASE,
            pages: text_pages + stack_pages,
            text_pages,
            frame: FRAME_SIZE,
        };
        if text_pages > MAX_TEXT_PAGES {
            return Err(AsError::OutOfRange);
        }
        window.validate().map_err(|_| AsError::OutOfRange)?;

        let asid = crate::mm::asid::alloc().ok_or(AsError::OutOfAsid)?;
        let (root, root_phys) = match frames::alloc() {
            Some(pair) => pair,
            None => {
                let _ = crate::mm::asid::free(asid);
                return Err(AsError::OutOfFrames);
            }
        };
        // SAFETY: identity-mapped pool frame exclusive to us.
        unsafe {
            zero_table(root_phys);
        }

        let mut owned = FrameLedger::new();
        if owned.push(root.index()).is_err() {
            let _ = frames::free(root);
            let _ = crate::mm::asid::free(asid);
            return Err(AsError::LedgerFull);
        }

        debug_assert_eq!(FRAME_SIZE as u64, PAGE_SIZE);

        Ok(Self {
            root_phys,
            asid,
            owned,
            user_sp: 0,
            text_phys: [0; MAX_TEXT_PAGES],
            window,
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
        // SAFETY: both roots are pool frames, identity-mapped RW for EL1, and
        // `kernel_root_phys` returned `Some` so the kernel map is live. The
        // clone reads a table the kernel is currently translating through and
        // writes one nothing has installed yet, so no walker can observe a
        // half-copied level.
        unsafe {
            self.clone_table_into(kroot as *const u64, self.root_phys as *mut u64, Level::L1)?;
        }
        self.map_user_window()?;
        self.prepared = true;
        Ok(())
    }

    /// Physical root for `TTBR0_EL1` (BADDR only, no ASID bits).
    #[inline]
    pub fn root_phys(&self) -> usize {
        self.root_phys
    }

    /// ASID assigned to this address space (never 0).
    #[inline]
    pub fn asid(&self) -> u16 {
        self.asid
    }

    /// Packed `TTBR0_EL1` value: physical root + ASID in bits [63:48].
    #[inline]
    pub fn ttbr0_value(&self) -> u64 {
        crate::mm::asid::pack_ttbr0(self.root_phys, self.asid)
    }

    /// Initial user SP (stack top) after prepare; 0 if not prepared.
    #[inline]
    pub fn user_sp(&self) -> u64 {
        self.user_sp
    }

    /// User entry VA (bottom of the text) after prepare.
    #[inline]
    pub fn user_entry_va(&self) -> u64 {
        if self.prepared {
            self.window.entry()
        } else {
            0
        }
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
        // SAFETY: this AS is not installed in `TTBR0` — it is prepared, not
        // live — so these tables have no walker. `pa` is a BSP-named device
        // page and the alignment of both addresses was checked above. Note what
        // is *not* checked: nothing says this agent was granted this device.
        // That is ADR-0016's missing capability ABI, not a gap in this call.
        unsafe { self.map_l3_page(va, pa, MemKind::Device, perms) }
    }

    /// Write raw bytes into the user **text**, page by page (kernel identity
    /// access to phys).
    ///
    /// After each page, publishes that range for instruction fetch (D clean to
    /// PoU + I invalidate). Required on Cortex-A72 whenever the bytes may run at
    /// EL0 — not optional for QEMU.
    ///
    /// The bound is the text, not the whole window: the stack pages above it are
    /// the agent's, and a write running past the text into them would be the
    /// kernel scribbling on a running program's stack.
    ///
    /// **Nothing here assumes the text is one contiguous run of physical
    /// memory.** Each page came from its own [`frames::alloc`], and they are
    /// adjacent only by accident of the pool's LIFO free order — an accident
    /// that holds on a fresh boot and stops holding after the first
    /// create/destroy cycle. So the copy is split at page boundaries and each
    /// piece goes to the physical address recorded for that page.
    pub fn poke_user(&self, offset: usize, bytes: &[u8]) -> Result<(), AsError> {
        if !self.prepared {
            return Err(AsError::BadTable);
        }
        self.window
            .bound_text_write(offset, bytes.len())
            .map_err(|_| AsError::OutOfRange)?;

        let frame = self.window.frame;
        let mut written = 0usize;
        while written < bytes.len() {
            let at = offset + written;
            let page = at / frame;
            let in_page = at % frame;
            // The bound above puts `at` inside the text, so `page` indexes a
            // mapped entry — asserted rather than assumed, because a zero here
            // would be a write to physical address `in_page`.
            let base = self.text_phys[page];
            if base == 0 {
                return Err(AsError::BadTable);
            }
            let take = (frame - in_page).min(bytes.len() - written);
            let dest = base + in_page;
            // SAFETY: `base` is a pool frame this AS owns, identity-mapped RW
            // for EL1, so the kernel may write it directly; `in_page + take`
            // cannot exceed `frame`, so the copy stays inside it.
            // `publish_executable` is required, not optional: these pages are
            // mapped `USER_RX` and the bytes are about to be fetched as
            // instructions on a core whose caches are not coherent.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr().add(written), dest as *mut u8, take);
                cache::publish_executable(dest, take);
            }
            written += take;
        }
        Ok(())
    }

    /// Physical address of text page `page` after prepare, `None` if unmapped.
    ///
    /// For callers that need to name a text location to a peer that cannot
    /// hold a reference to this AS (ADR-0064 stop word): the peer writes
    /// through the kernel identity alias, exactly as [`Self::poke_user`] does.
    #[inline]
    pub fn text_page_phys(&self, page: usize) -> Option<usize> {
        match self.text_phys.get(page) {
            Some(&pa) if pa != 0 => Some(pa),
            _ => None,
        }
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

    /// Free every owned frame and return the ASID. Consumes the AS.
    ///
    /// Invalidates TLB entries tagged with this ASID before the tag is reused
    /// (K7 / ADR-0050). See note 2 at the top of this file.
    pub fn destroy(mut self) {
        for &index in self.owned.as_slice() {
            let _ = frames::free(FrameId::from_index(index));
        }
        self.owned.clear();
        let asid = self.asid;
        // Invalidate before free so a concurrent alloc cannot install the tag
        // while stale entries remain (ASID pool is IrqSpinLock-serialised).
        mmu::invalidate_asid(asid);
        let _ = crate::mm::asid::free(asid);
        core::mem::forget(self);
    }

    /// Map the private user window at [`USER_VA_BASE`].
    ///
    /// Layout: the lowest `text_pages` are user text (`USER_RX`), the rest are
    /// stack (`USER_RW`), and `SP_EL0` starts at the top. W^X holds inside the
    /// window as it does in the kernel map — [`UserWindow::is_text_page`] is the
    /// one place that decides which is which, and it is host-tested. Kernel
    /// leaves share PA and keep EL0-denied AP from the clone step.
    fn map_user_window(&mut self) -> Result<(), AsError> {
        let mut va = self.window.base;
        for i in 0..self.window.pages {
            let (id, phys) = frames::alloc().ok_or(AsError::OutOfFrames)?;
            self.track(id)?;
            if self.window.is_text_page(i) {
                self.text_phys[i] = phys;
            }
            // SAFETY: `frames::alloc` just returned this frame, so nothing
            // else holds it, and pool frames are identity-mapped RW for EL1.
            // Zeroing is what stops a user program from reading whatever the
            // previous owner of the frame left behind.
            unsafe {
                core::ptr::write_bytes(phys as *mut u8, 0, FRAME_SIZE);
            }
            let perms = if self.window.is_text_page(i) {
                Perms::USER_RX
            } else {
                Perms::USER_RW
            };
            // SAFETY: as `map_device_page` — tables of an AS that is not yet
            // installed, so no walker can see a partial update.
            unsafe {
                self.map_l3_page(va, phys as u64, MemKind::NormalWb, perms)?;
            }
            va += PAGE_SIZE;
        }
        self.user_sp = self.window.stack_top();
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
        // SAFETY: the walk starts at this AS's own root — a pool frame,
        // identity-mapped RW for EL1 — and every step follows a descriptor the
        // previous read proved to be a table, so no dereference leaves the
        // tables this AS owns. The AS is prepared, not installed, so no walker
        // can observe an intermediate state. Volatile accessors are used
        // throughout because these words are also read by hardware.
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
        // SAFETY: `src` is the live kernel root or a table reached from it, and
        // `dst` is a frame this AS owns and nothing has installed. Both are
        // identity-mapped tables of `ENTRIES_PER_TABLE` words, which bounds the
        // loop. The recursion follows only descriptors `is_leaf` rejected, so
        // it cannot walk into a block's output address as though it were a
        // table.
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
        // Mirror `destroy()`: a dropped AS must not leak its ASID or leave
        // stale tagged TLB entries behind (ADR-0050). The two teardown paths
        // diverged once — an early-return that skipped `destroy()` would have
        // burned an ASID from an 8-bit pool in silence.
        mmu::invalidate_asid(self.asid);
        let _ = crate::mm::asid::free(self.asid);
    }
}

/// Blank one translation table.
///
/// # Safety
/// `phys` is a page-aligned frame the caller owns and nothing has installed.
unsafe fn zero_table(phys: usize) {
    // SAFETY: a freshly allocated pool frame, identity-mapped RW for EL1 and
    // not reachable by any walker yet. The loop writes exactly the
    // `ENTRIES_PER_TABLE` words a table is made of, so it stays inside the
    // frame. Every entry must be cleared before use: `alloc_table` in the
    // kernel arena has the same requirement, because a stale non-zero word
    // reads as a valid descriptor.
    unsafe {
        let table = phys as *mut u64;
        for i in 0..ENTRIES_PER_TABLE {
            core::ptr::write_volatile(table.add(i), 0);
        }
    }
}
