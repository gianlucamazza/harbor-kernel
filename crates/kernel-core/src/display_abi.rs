//! Read-only descriptor shared with the EL0 screen agent.

use crate::mailbox::{BYTES_PER_PIXEL, DEPTH, HEIGHT, WIDTH};

pub const MAGIC: u32 = 0x4652_4D31;
pub const VERSION: u32 = 1;
pub const FORMAT_RGB565: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramebufferDescriptor {
    pub magic: u32,
    pub version: u32,
    pub address: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub depth: u32,
    pub format: u32,
    pub reserved: [u32; 5],
}

impl FramebufferDescriptor {
    pub const fn new(address: u64, size: u64, pitch: u32) -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            address,
            size,
            width: WIDTH,
            height: HEIGHT,
            pitch,
            depth: DEPTH,
            format: FORMAT_RGB565,
            reserved: [0; 5],
        }
    }

    pub const fn validate(&self) -> bool {
        let mut reserved_zero = true;
        let mut i = 0;
        while i < self.reserved.len() {
            reserved_zero &= self.reserved[i] == 0;
            i += 1;
        }
        let end = self.address.checked_add(self.size);
        self.magic == MAGIC
            && self.version == VERSION
            && self.address != 0
            && self.address.is_multiple_of(crate::paging::PAGE_SIZE)
            && end.is_some()
            && self.width == WIDTH
            && self.height == HEIGHT
            && self.depth == DEPTH
            && self.format == FORMAT_RGB565
            && self.pitch >= WIDTH * BYTES_PER_PIXEL
            && self.size >= self.pitch as u64 * self.height as u64
            && reserved_zero
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_round_trips_and_validates() {
        let d = FramebufferDescriptor::new(0x1000, 2 * 1024 * 1024, WIDTH * 2);
        assert!(d.validate());
    }

    #[test]
    fn malformed_descriptor_is_rejected() {
        let mut d = FramebufferDescriptor::new(0x1000, 2 * 1024 * 1024, WIDTH * 2);
        d.pitch = WIDTH;
        assert!(!d.validate());
    }

    #[test]
    fn descriptor_rejects_unaligned_or_overflowing_ranges() {
        let mut unaligned = FramebufferDescriptor::new(0x1001, 2 * 1024 * 1024, WIDTH * 2);
        assert!(!unaligned.validate());
        unaligned.address = u64::MAX - 0x1000;
        assert!(!unaligned.validate());
    }
}
