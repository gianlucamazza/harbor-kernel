//! What the firmware told us at entry.
//!
//! On AArch64 the boot protocol hands the kernel a device-tree blob address in
//! `x0`. `boot.s` stashes it before that register is reused. Nothing parses it
//! yet, but every hard-coded constant in the BSP — RAM size, UART reference
//! clock, peripheral base — is a fact the DTB already knows, and the pointer is
//! unrecoverable once clobbered.
//!
//! # Validated once, then never dereferenced again
//!
//! Checking the magic means reading the blob, and the firmware places it
//! wherever it likes: `0x2eff1f00` on this board, `0x8000000` under QEMU —
//! both far outside the regions `mm::layout` maps. So the check is only
//! possible while the coarse early map is active, and [`survey`] is called
//! during bring-up to do it exactly once.
//!
//! Everything afterwards reads the cached answer. That is deliberate: the
//! alternative is a function whose correctness depends on *when* you call it,
//! documented in three places — which is precisely the shape of the bug that
//! cost this project an afternoon. Whoever first wants to parse the blob will
//! have to map it, and mapping it is a visible decision rather than an
//! assumption about ordering.

use core::sync::atomic::{AtomicU64, Ordering};

use kernel_core::paging::PAGE_SIZE;

unsafe extern "C" {
    /// Written by `_start` from `x0`, before BSS is cleared.
    static __dtb_ptr: u64;
}

/// Magic at the start of a flattened device tree (`FDT_MAGIC`, big endian).
const FDT_MAGIC: u32 = 0xd00d_feed;

/// Validated blob address, or 0 for "no device tree". Set by [`survey`].
static DEVICE_TREE: AtomicU64 = AtomicU64::new(0);

/// Total size of the blob, from its header. Read during [`survey`], while the
/// early map still covers wherever the firmware put it.
static DEVICE_TREE_LEN: AtomicU64 = AtomicU64::new(0);

/// The raw value the firmware left in `x0`, whether or not it is a DTB.
///
/// Reported when [`device_tree`] is `None`, so a board that passes something
/// unexpected shows what it passed instead of just "nothing".
pub fn dtb_address() -> u64 {
    // SAFETY: plain read of a `.data` word written once before `kernel_main`.
    unsafe { core::ptr::read_volatile(&raw const __dtb_ptr) }
}

/// Inspect what the firmware passed and cache the result.
///
/// # Safety
///
/// Must run while every physical address is readable — during bring-up, before
/// [`crate::arch::mmu::activate`] narrows the map. Call once.
pub unsafe fn survey() {
    let address = dtb_address();
    if address == 0 || !address.is_multiple_of(8) {
        return;
    }

    // SAFETY: a 4-byte read at an 8-aligned address covered by the early map.
    // A wrong guess from the firmware faults visibly rather than corrupting.
    let magic = unsafe { core::ptr::read_volatile(address as *const u32) };
    if u32::from_be(magic) != FDT_MAGIC {
        return;
    }

    // `totalsize` is the second big-endian word of the header. Read it now,
    // for the same reason as the magic: after `mmu::activate` the blob is
    // outside every mapped region, so this is the last chance without a
    // mapping — and the mapping needs the size.
    // SAFETY: the header is 8 bytes; the magic just proved this is one.
    let total = unsafe { core::ptr::read_volatile((address + 4) as *const u32) };
    let len = u64::from(u32::from_be(total));
    if len == 0 {
        return;
    }

    DEVICE_TREE_LEN.store(len, Ordering::Release);
    DEVICE_TREE.store(address, Ordering::Release);
}

/// The blob's address and length, page-aligned outwards so the pair can be
/// handed straight to [`crate::arch::mmu::map`].
pub fn device_tree_pages() -> Option<(u64, u64)> {
    let address = device_tree()?;
    let len = DEVICE_TREE_LEN.load(Ordering::Acquire);

    let base = address & !(PAGE_SIZE - 1);
    let end = (address + len).next_multiple_of(PAGE_SIZE);
    Some((base, end - base))
}

/// The device-tree address, if the firmware passed something that really is
/// one. Safe to call at any point after [`survey`]; dereferences nothing.
pub fn device_tree() -> Option<u64> {
    match DEVICE_TREE.load(Ordering::Acquire) {
        0 => None,
        address => Some(address),
    }
}

/// The blob as a byte slice — the consume half of the facade contract
/// ("early map + optional consume", ADR-0072/0073).
///
/// # Safety
///
/// The caller must have mapped the blob (the RO map `bootstrap` builds from
/// [`device_tree_pages`]) and it must stay mapped for `'static`. Before that
/// map exists this address is unmapped and the first read faults.
pub unsafe fn device_tree_slice() -> Option<&'static [u8]> {
    let address = device_tree()?;
    let len = DEVICE_TREE_LEN.load(Ordering::Acquire);
    // SAFETY: caller upholds the mapping; length comes from the validated
    // header and the region is mapped RO, so no one mutates it.
    Some(unsafe { core::slice::from_raw_parts(address as *const u8, len as usize) })
}
