//! QEMU AArch64 `virt` memory map (GICv2 configuration).
//!
//! These addresses are the platform contract emitted by QEMU's DTB when
//! `-machine virt,gic-version=2` is selected. The DTB remains discovery input;
//! constants are the compiled claims the boot report reconciles against it.

use kernel_core::paging::MemKind;

pub const FRAME_SIZE: usize = 0x1000;
pub const RAM_START: usize = 0x4000_0000;

/// Coarse early map: low device space, then the 128 MiB RAM aperture at 1 GiB.
pub const EARLY_BLOCKS: [MemKind; 4] = {
    use MemKind::{Device, NormalWb};
    [Device, NormalWb, NormalWb, NormalWb]
};

pub const UART0_BASE: usize = 0x0900_0000;
pub const UART0_REG_BYTES: usize = FRAME_SIZE;
pub const UART0_CLOCK_HZ: u32 = 24_000_000;
pub const UART0_BAUD: u32 = 115_200;

pub const GICD_BASE: usize = 0x0800_0000;
pub const GICC_BASE: usize = 0x0801_0000;
pub const TIMER_PPI: u32 = 30;
pub const UART0_SPI: u32 = 33;
pub const VIRTIO_NET_BASE: usize = 0x0A00_0000;
pub const VIRTIO_NET_REG_BYTES: usize = 0x200;
pub const VIRTIO_MMIO_STRIDE: usize = 0x200;
pub const VIRTIO_MMIO_SLOTS: usize = 32;

/// QEMU `virt` memory starts at 1 GiB; this product maps its first 128 MiB.
pub const IDENTITY_RAM_END: usize = 0x4800_0000;
pub const EXPECTED_MODEL_PREFIX: &str = "linux,dummy-virt";
pub const EXPECTED_CPUS: u32 = 4;
pub const FRAME_POOL_FRAMES: usize = 512;
pub const FRAME_POOL_BYTES: usize = FRAME_POOL_FRAMES * FRAME_SIZE;

pub const USER_VA_BASE: u64 = 0x0000_0000_5000_0000;
pub const USER_STACK_PAGES: usize = 4;
pub const USER_PL011_VA: u64 = 0x0000_0000_5100_0000;
pub const RNG200_BASE: usize = 0;
pub const RNG200_REG_BYTES: usize = FRAME_SIZE;
pub const USER_RNG_VA: u64 = 0x0000_0000_5200_0000;
pub const USER_PACKET_POOL_VA: u64 = 0x0000_0000_5300_0000;

pub const DEVICE_REGIONS: [(usize, usize, &str); 3] = [
    (GICD_BASE, 0x0002_0000, "GICv2"),
    (UART0_BASE, 0x0000_1000, "PL011"),
    (
        VIRTIO_NET_BASE,
        VIRTIO_NET_REG_BYTES * VIRTIO_MMIO_SLOTS,
        "virtio-mmio",
    ),
];

const _: () = {
    let mut i = 0;
    while i < DEVICE_REGIONS.len() {
        assert!(DEVICE_REGIONS[i].0.is_multiple_of(FRAME_SIZE));
        assert!(DEVICE_REGIONS[i].1.is_multiple_of(FRAME_SIZE));
        assert!(DEVICE_REGIONS[i].1 > 0);
        if i + 1 < DEVICE_REGIONS.len() {
            assert!(DEVICE_REGIONS[i].0 + DEVICE_REGIONS[i].1 <= DEVICE_REGIONS[i + 1].0);
        }
        i += 1;
    }
};
