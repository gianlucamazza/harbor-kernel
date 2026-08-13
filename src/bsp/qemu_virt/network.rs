//! QEMU virtio-net board bind.

use crate::arch::mmio::Mmio;
use crate::drivers::virtio_mmio::{self, Negotiated, ProbeFailure};

/// Probe the first virtio-mmio slot described by the QEMU virt contract.
///
/// # Safety
/// Called after the kernel map is active and before another network owner is
/// installed.
pub unsafe fn probe() -> Result<(usize, Negotiated), ProbeFailure> {
    let mut last_error = None;
    for slot in 0..super::memmap::VIRTIO_MMIO_SLOTS {
        let base = super::memmap::VIRTIO_NET_BASE + slot * super::memmap::VIRTIO_MMIO_STRIDE;
        // SAFETY: every QEMU virt slot lies in the mapped virtio-mmio aperture.
        match unsafe { virtio_mmio::probe(Mmio::new(base)) } {
            Ok(negotiated) => return Ok((base, negotiated)),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.expect("QEMU virt has at least one virtio-mmio slot"))
}
