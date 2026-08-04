//! What the firmware told us at entry.
//!
//! On AArch64 the boot protocol hands the kernel a device-tree blob address in
//! `x0`. `boot.s` stashes it before that register is reused. Nothing consumes
//! it yet, but every hard-coded constant in the BSP — RAM size, UART reference
//! clock, peripheral base — is a fact the DTB already knows, and the pointer is
//! unrecoverable once clobbered.

unsafe extern "C" {
    /// Written by `_start` from `x0`, before BSS is cleared.
    static __dtb_ptr: u64;
}

/// Magic at the start of a flattened device tree (`FDT_MAGIC`, big endian).
const FDT_MAGIC: u32 = 0xd00d_feed;

/// The raw value the firmware left in `x0`.
pub fn dtb_address() -> u64 {
    // SAFETY: plain read of a `.data` word written once before `kernel_main`.
    unsafe { core::ptr::read_volatile(&raw const __dtb_ptr) }
}

/// The device-tree address, if one that actually looks like a DTB was passed.
///
/// Checked rather than trusted: firmware may pass zero, and QEMU's `-kernel`
/// path passes an address only for some machines.
///
/// # Call this before the MMU is enabled
///
/// Validating the magic dereferences the address, and the firmware is free to
/// place the DTB anywhere in RAM — under QEMU it lands at `0x8000000`, well
/// outside the regions `mm::layout` maps. With translation off every physical
/// address is readable; afterwards this would take a translation fault.
/// Mapping the blob is work for whoever first parses it.
pub fn device_tree() -> Option<u64> {
    let address = dtb_address();
    if address == 0 || address % 8 != 0 {
        return None;
    }

    // SAFETY: the header is 4 bytes at an 8-aligned address inside identity
    // mapped low RAM; a wrong guess faults visibly rather than corrupting.
    let magic = unsafe { core::ptr::read_volatile(address as *const u32) };
    if u32::from_be(magic) == FDT_MAGIC {
        Some(address)
    } else {
        None
    }
}
