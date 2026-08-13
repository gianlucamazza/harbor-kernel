//! Modern virtio-mmio transport probe (ADR-0104).
//!
//! This is deliberately a probe, not a partial network driver: it validates
//! the transport, negotiates only VERSION_1, verifies the device's
//! `FEATURES_OK` acknowledgement, then resets before returning. Queue memory,
//! interrupt ownership, and packet service become a separate change with a
//! complete lifecycle.

use kernel_core::virtio::{self, DeviceInfo, DriverStatus, FeatureError, ProbeError, StatusError};

use crate::arch::mmio::Mmio;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeFailure {
    Identity(ProbeError),
    Features(FeatureError),
    Status(StatusError),
    DeviceClearedFeatures,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Negotiated {
    pub device: DeviceInfo,
    pub features: u64,
}

/// Probe one modern virtio-mmio slot and leave it reset.
///
/// # Safety
/// `mmio` must cover one valid virtio-mmio register window mapped as Device
/// memory, and no other driver may access the slot concurrently.
pub unsafe fn probe(mmio: Mmio) -> Result<Negotiated, ProbeFailure> {
    let magic = mmio.read32(virtio::mmio::MAGIC_VALUE);
    let version = mmio.read32(virtio::mmio::VERSION);
    let device_id = mmio.read32(virtio::mmio::DEVICE_ID);
    let vendor = mmio.read32(virtio::mmio::VENDOR_ID);
    let features = read_features(mmio);
    let device = virtio::probe_device(magic, version, device_id, vendor, features)
        .map_err(ProbeFailure::Identity)?;

    let negotiated = virtio::negotiate_features(device.features, virtio::FEATURE_VERSION_1, 0)
        .map_err(ProbeFailure::Features)?;

    let mut status = DriverStatus::new();
    status.acknowledge().map_err(ProbeFailure::Status)?;
    write_status(mmio, status.bits());
    status.driver().map_err(ProbeFailure::Status)?;
    write_status(mmio, status.bits());
    write_features(mmio, negotiated);
    status
        .features_ok(negotiated)
        .map_err(ProbeFailure::Status)?;
    write_status(mmio, status.bits());

    if mmio.read32(virtio::mmio::STATUS) as u8 & virtio::status::FEATURES_OK == 0 {
        write_status(mmio, 0);
        return Err(ProbeFailure::DeviceClearedFeatures);
    }

    // No queue has been selected or exposed, so DRIVER_OK would be an invalid
    // readiness claim. Reset makes this probe side-effect free for the future
    // queue owner and is required on every failure path after recognition.
    write_status(mmio, 0);
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
