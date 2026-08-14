//! QEMU virtio-net board bind.

use crate::arch::mmio::Mmio;
use crate::drivers::virtio_mmio::{self, Negotiated, QueueMemory, QueueSetupFailure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueReport {
    pub base: usize,
    pub negotiated: Negotiated,
    pub queues: usize,
    pub queue_size: usize,
}

/// Find the QEMU virtio-net slot and configure both split queues.
///
/// The caller retains the returned object and owns its reset lifecycle.
pub unsafe fn configure(
    rings: [QueueMemory; 2],
) -> Result<(QueueReport, virtio_mmio::Configured), QueueSetupFailure> {
    let mut last_error = None;
    for slot in 0..super::memmap::VIRTIO_MMIO_SLOTS {
        let base = super::memmap::VIRTIO_NET_BASE + slot * super::memmap::VIRTIO_MMIO_STRIDE;
        // SAFETY: every QEMU virt slot lies in the mapped virtio-mmio aperture.
        match unsafe { virtio_mmio::configure(Mmio::new(base), rings) } {
            Ok(configured) => {
                let report = QueueReport {
                    base,
                    negotiated: configured.negotiated(),
                    queues: configured.queue_count(),
                    queue_size: configured.queue_size(),
                };
                return Ok((report, configured));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.expect("QEMU virt has at least one virtio-mmio slot"))
}
