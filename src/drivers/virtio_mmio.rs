//! Modern virtio-mmio transport probe (ADR-0104).
//!
//! This is deliberately a probe, not a partial network driver: it validates
//! the transport, negotiates only VERSION_1, verifies the device's
//! `FEATURES_OK` acknowledgement, then resets before returning. Queue memory,
//! interrupt ownership, and packet service become a separate change with a
//! complete lifecycle.

use kernel_core::virtio::{self, DeviceInfo, DriverStatus, FeatureError, ProbeError, StatusError};

use crate::arch::mmio::Mmio;

const QUEUE_COUNT: usize = 2;
const QUEUE_SIZE: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueSetupFailure {
    Identity(ProbeError),
    Features(FeatureError),
    Status(StatusError),
    DeviceClearedFeatures,
    QueueTooSmall { queue: u32, maximum: u32 },
    QueueNotReady(u32),
    DriverNotReady(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Negotiated {
    pub device: DeviceInfo,
    pub features: u64,
}

/// Physical addresses of the three split-ring regions for one queue.
///
/// The allocator and cache policy remain above the driver layer. The driver
/// receives only these already-owned DMA addresses and never knows which
/// allocator or board supplied them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueMemory {
    pub desc_pa: u64,
    pub avail_pa: u64,
    pub used_pa: u64,
}

/// A configured but not yet service-owned transport.
///
/// The object owns the transport status lifecycle. The caller owns the DMA
/// frame lifetime and must release those frames after calling `reset`.
pub struct Configured {
    mmio: Mmio,
    negotiated: Negotiated,
}

impl Configured {
    pub const fn queue_count(&self) -> usize {
        QUEUE_COUNT
    }

    pub const fn queue_size(&self) -> usize {
        QUEUE_SIZE
    }

    pub const fn negotiated(&self) -> Negotiated {
        self.negotiated
    }

    /// Reset the device and release every EL1-owned ring frame.
    pub fn reset(&mut self) {
        write_status(self.mmio, 0);
    }
}

/// Acknowledge one virtio-mmio interrupt using the trusted transport base
/// carried by the opaque IRQ cookie. Queue consumption remains a service
/// concern; the IRQ path never allocates, blocks, or switches tasks.
pub fn on_irq(cookie: u32) {
    let base = cookie as usize;
    if base == 0 {
        return;
    }
    // SAFETY: only the BSP registers this handler and supplies a mapped
    // virtio-mmio base as its cookie.
    let mmio = unsafe { Mmio::new(base) };
    let status = mmio.read32(virtio::mmio::INTERRUPT_STATUS);
    if status != 0 {
        mmio.write32(virtio::mmio::INTERRUPT_ACK, status);
    }
}

/// Configure both virtio-net split queues using six EL1-owned frame pages.
///
/// # Safety
/// `mmio` must be a valid, exclusively-owned modern virtio-mmio network slot;
/// the frame pool must already be initialised and the MMIO window mapped as
/// Device memory.
pub unsafe fn configure(
    mmio: Mmio,
    rings: [QueueMemory; QUEUE_COUNT],
) -> Result<Configured, QueueSetupFailure> {
    let negotiated = begin_transport(mmio)?;

    for (queue, ring) in rings.iter().enumerate() {
        let queue = queue as u32;
        mmio.write32(virtio::mmio::QUEUE_SEL, queue);
        let maximum = mmio.read32(virtio::mmio::QUEUE_NUM_MAX);
        if maximum < QUEUE_SIZE as u32 {
            write_status(mmio, 0);
            return Err(QueueSetupFailure::QueueTooSmall { queue, maximum });
        }

        mmio.write32(virtio::mmio::QUEUE_NUM, QUEUE_SIZE as u32);
        mmio.write32(virtio::mmio::QUEUE_DESC_LOW, ring.desc_pa as u32);
        mmio.write32(virtio::mmio::QUEUE_DESC_HIGH, (ring.desc_pa >> 32) as u32);
        mmio.write32(virtio::mmio::QUEUE_AVAIL_LOW, ring.avail_pa as u32);
        mmio.write32(virtio::mmio::QUEUE_AVAIL_HIGH, (ring.avail_pa >> 32) as u32);
        mmio.write32(virtio::mmio::QUEUE_USED_LOW, ring.used_pa as u32);
        mmio.write32(virtio::mmio::QUEUE_USED_HIGH, (ring.used_pa >> 32) as u32);
        mmio.write32(virtio::mmio::QUEUE_READY, 1);
        if mmio.read32(virtio::mmio::QUEUE_READY) != 1 {
            write_status(mmio, 0);
            return Err(QueueSetupFailure::QueueNotReady(queue));
        }
    }

    let status = virtio::status::ACKNOWLEDGE
        | virtio::status::DRIVER
        | virtio::status::FEATURES_OK
        | virtio::status::DRIVER_OK;
    mmio.write32(virtio::mmio::STATUS, u32::from(status));
    let observed = mmio.read32(virtio::mmio::STATUS) as u8;
    if observed & virtio::status::DRIVER_OK == 0 {
        write_status(mmio, 0);
        return Err(QueueSetupFailure::DriverNotReady(observed));
    }

    Ok(Configured { mmio, negotiated })
}

fn begin_transport(mmio: Mmio) -> Result<Negotiated, QueueSetupFailure> {
    write_status(mmio, 0);
    let magic = mmio.read32(virtio::mmio::MAGIC_VALUE);
    let version = mmio.read32(virtio::mmio::VERSION);
    let device_id = mmio.read32(virtio::mmio::DEVICE_ID);
    let vendor = mmio.read32(virtio::mmio::VENDOR_ID);
    let features = read_features(mmio);
    let device = virtio::probe_device(magic, version, device_id, vendor, features)
        .map_err(QueueSetupFailure::Identity)?;
    let negotiated = virtio::negotiate_features(device.features, virtio::FEATURE_VERSION_1, 0)
        .map_err(QueueSetupFailure::Features)?;

    let mut status = DriverStatus::new();
    status.acknowledge().map_err(QueueSetupFailure::Status)?;
    write_status(mmio, status.bits());
    status.driver().map_err(QueueSetupFailure::Status)?;
    write_status(mmio, status.bits());
    write_features(mmio, negotiated);
    status
        .features_ok(negotiated)
        .map_err(QueueSetupFailure::Status)?;
    write_status(mmio, status.bits());
    if mmio.read32(virtio::mmio::STATUS) as u8 & virtio::status::FEATURES_OK == 0 {
        write_status(mmio, 0);
        return Err(QueueSetupFailure::DeviceClearedFeatures);
    }
    Ok(Negotiated {
        device,
        features: negotiated,
    })
}

fn read_features(mmio: Mmio) -> u64 {
    mmio.write32(virtio::mmio::DEVICE_FEATURES_SEL, 0);
    let low = mmio.read32(virtio::mmio::DEVICE_FEATURES) as u64;
    mmio.write32(virtio::mmio::DEVICE_FEATURES_SEL, 1);
    let high = mmio.read32(virtio::mmio::DEVICE_FEATURES) as u64;
    low | high << 32
}

fn write_features(mmio: Mmio, features: u64) {
    mmio.write32(virtio::mmio::DRIVER_FEATURES_SEL, 0);
    mmio.write32(virtio::mmio::DRIVER_FEATURES, features as u32);
    mmio.write32(virtio::mmio::DRIVER_FEATURES_SEL, 1);
    mmio.write32(virtio::mmio::DRIVER_FEATURES, (features >> 32) as u32);
}

fn write_status(mmio: Mmio, status: u8) {
    mmio.write32(virtio::mmio::STATUS, u32::from(status));
}
