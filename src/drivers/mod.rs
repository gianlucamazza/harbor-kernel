//! Hardware drivers.
//!
//! Drivers are board-agnostic. The BSP supplies addresses, clocks, and pinmux.

#[cfg(feature = "debug-display")]
pub mod delay;
#[cfg(target_arch = "aarch64")]
#[cfg_attr(
    feature = "board-rpi4",
    expect(
        dead_code,
        reason = "GENET leftover dataplane compiled; probe/identify/link/queue0/rgmii/umac/tbuf/mib selected"
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
#[cfg(feature = "debug-display")]
pub mod ili9486;
#[cfg(target_arch = "aarch64")]
pub mod mailbox;
#[cfg(feature = "debug-display")]
pub mod pin;
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
#[cfg(feature = "debug-display")]
pub mod spi;
#[cfg(feature = "display-touch")]
pub mod touch;
#[cfg(all(target_arch = "aarch64", feature = "board-qemu-virt"))]
pub mod virtio_mmio;

#[cfg(target_arch = "x86_64")]
pub mod uart16550;
