//! Hardware drivers.
//!
//! Drivers are board-agnostic. The BSP supplies addresses, clocks, and pinmux.

#[cfg(target_arch = "aarch64")]
#[cfg_attr(
    feature = "board-rpi4",
    expect(
        dead_code,
        reason = "GENET leftover dataplane compiled; probe/identify/link/queue0/rgmii/umac/tbuf/desc-ring/mib selected"
    )
)]
#[cfg_attr(
    feature = "board-qemu-virt",
    expect(
        dead_code,
        reason = "GENET control plane is compiled but qemu-virt has no GENET"
    )
)]
pub mod genet;
#[cfg(target_arch = "aarch64")]
pub mod gicv2;
#[cfg(target_arch = "aarch64")]
pub mod pl011;
#[cfg(target_arch = "aarch64")]
#[cfg_attr(
    feature = "board-qemu-virt",
    expect(
        dead_code,
        reason = "QEMU virt keeps the shared board-driver API absent"
    )
)]
pub mod pm;
#[cfg(target_arch = "aarch64")]
#[cfg_attr(
    feature = "board-qemu-virt",
    expect(
        dead_code,
        reason = "QEMU virt keeps the shared board-driver API absent"
    )
)]
pub mod rng200;
#[cfg(target_arch = "aarch64")]
#[cfg_attr(
    feature = "board-qemu-virt",
    expect(
        dead_code,
        reason = "QEMU virt keeps the shared board-driver API absent"
    )
)]
pub mod sdhci;
#[cfg(all(target_arch = "aarch64", feature = "board-qemu-virt"))]
pub mod virtio_mmio;

#[cfg(target_arch = "x86_64")]
pub mod uart16550;
