//! Raspberry Pi 4 framebuffer discovery through the VideoCore mailbox.

use kernel_core::held::Window;
use kernel_core::mailbox::FramebufferInfo;
use kernel_core::paging::Perms;

use crate::drivers::mailbox::{Error, PropertyMailbox};

/// Result of a successful firmware framebuffer allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Framebuffer {
    pub info: FramebufferInfo,
    pub window: Window,
}

/// Discover the firmware-owned framebuffer without exposing the mailbox.
pub fn probe() -> Result<Framebuffer, Error> {
    // SAFETY: the RPi4 BSP owns the mailbox property channel during bootstrap;
    // no agent receives the register window or the request buffer.
    let mailbox = unsafe { PropertyMailbox::new(super::memmap::MAILBOX_BASE) };
    let info = mailbox.framebuffer()?;
    // The firmware's framebuffer response is a bus address. Raspberry Pi 4's
    // ARM-visible alias is the low 1 GiB physical address used by the identity
    // map; reject a response that cannot be represented as that range.
    let pa = info.address & 0x3fff_ffff;
    let len = u64::from(info.size).div_ceil(kernel_core::paging::PAGE_SIZE)
        * kernel_core::paging::PAGE_SIZE;
    let window = Window {
        pa,
        len,
        perms: Perms::USER_RW,
    };
    if !window.is_valid() || pa + len > 0x4000_0000 {
        return Err(Error::Contract(kernel_core::mailbox::Error::InvalidSize));
    }
    Ok(Framebuffer { info, window })
}
