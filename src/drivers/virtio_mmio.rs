//! Modern virtio-mmio transport and bounded split-queue driver (ADR-0104).
//!
//! MMIO, descriptor rings, and DMA addresses remain EL1-only. Bootstrap owns
//! allocation and cache maintenance; this layer owns the transport protocol
//! and validates every descriptor operation before touching device state.

use core::sync::atomic::{Ordering, fence};
use kernel_core::virtio::{self, DeviceInfo, DriverStatus, FeatureError, ProbeError, StatusError};

use crate::arch::mmio::Mmio;

const QUEUE_COUNT: usize = 2;
const QUEUE_SIZE: usize = 8;
const RX_QUEUE: usize = 0;
const TX_QUEUE: usize = 1;
const PACKET_BYTES: usize = kernel_core::virtio::PACKET_BYTES;
const VIRTIO_NET_HEADER_BYTES: usize = 12;
const MAX_BUFFER_BYTES: usize = PACKET_BYTES + VIRTIO_NET_HEADER_BYTES;
const DESC_BYTES: usize = 16;
const AVAIL_HEADER_BYTES: usize = 4;
const USED_HEADER_BYTES: usize = 4;
const USED_ELEMENT_BYTES: usize = 8;
const DESC_F_WRITE: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueSetupFailure {
    Identity(ProbeError),
    Features(FeatureError),
    Status(StatusError),
    DeviceClearedFeatures,
    QueueTooSmall { queue: u32, maximum: u32 },
    QueueNotReady(u32),
    DriverNotReady(u8),
    InvalidRing { queue: u32 },
    InvalidBuffer,
    QueueFull { queue: u32 },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsedElement {
    pub descriptor: u16,
    pub len: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QueueCursor {
    avail: u16,
    used: u16,
}

/// A configured but not yet service-owned transport.
///
/// The object owns the transport status lifecycle. The caller owns the DMA
/// frame lifetime and must release those frames after calling `reset`.
pub struct Configured {
    mmio: Mmio,
    negotiated: Negotiated,
    rings: [QueueMemory; QUEUE_COUNT],
    cursors: [QueueCursor; QUEUE_COUNT],
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

    /// Queue 0 is RX and queue 1 is TX in the virtio-net contract.
    pub const fn rx_queue() -> usize {
        RX_QUEUE
    }

    pub const fn tx_queue() -> usize {
        TX_QUEUE
    }

    /// Reset the device and discard both queue cursors.
    pub fn reset(&mut self) {
        write_status(self.mmio, 0);
        self.cursors = [QueueCursor { avail: 0, used: 0 }; QUEUE_COUNT];
    }

    /// Re-negotiate the transport and rebind the retained split queues.
    ///
    /// Recovery is explicit and complete: the device returns through the
    /// modern transport handshake, queue addresses are written again, and
    /// `DRIVER_OK` is observed before the caller can publish buffers.
    pub fn restart(&mut self) -> Result<(), QueueSetupFailure> {
        let negotiated = begin_transport(self.mmio)?;
        let cursors = configure_queues(self.mmio, self.rings)?;
        self.negotiated = negotiated;
        self.cursors = cursors;
        Ok(())
    }

    /// Publish one receive buffer and notify the device.
    pub fn post_rx(&mut self, buffer_pa: u64, len: usize) -> Result<(), QueueSetupFailure> {
        self.publish(RX_QUEUE, buffer_pa, len, DESC_F_WRITE)
    }

    /// Publish one transmit buffer and notify the device.
    pub fn submit_tx(&mut self, buffer_pa: u64, len: usize) -> Result<(), QueueSetupFailure> {
        self.publish(TX_QUEUE, buffer_pa, len, 0)
    }

    /// Consume one device-completed descriptor, if available.
    pub fn poll_used(&mut self, queue: usize) -> Result<Option<UsedElement>, QueueSetupFailure> {
        let cursor = self
            .cursors
            .get_mut(queue)
            .ok_or(QueueSetupFailure::InvalidRing {
                queue: queue as u32,
            })?;
        let ring = self.rings[queue];
        // SAFETY: the ring was validated and retained by configure; the
        // device is the sole writer of used.idx and used elements.
        let used_idx = unsafe { read_u16(ring.used_pa + 2) };
        if cursor.used == used_idx {
            return Ok(None);
        }
        fence(Ordering::Acquire);
        let slot = (cursor.used as usize) & (QUEUE_SIZE - 1);
        // SAFETY: the ring was validated and retained by configure; the
        // device is the sole writer of used elements.
        let element = unsafe {
            UsedElement {
                descriptor: read_u32(
                    ring.used_pa + USED_HEADER_BYTES as u64 + (slot * USED_ELEMENT_BYTES) as u64,
                ) as u16,
                len: read_u32(
                    ring.used_pa
                        + USED_HEADER_BYTES as u64
                        + (slot * USED_ELEMENT_BYTES + 4) as u64,
                ),
            }
        };
        cursor.used = cursor.used.wrapping_add(1);
        Ok(Some(element))
    }

    fn publish(
        &mut self,
        queue: usize,
        buffer_pa: u64,
        len: usize,
        flags: u16,
    ) -> Result<(), QueueSetupFailure> {
        if virtio::validate_descriptor(buffer_pa, len, MAX_BUFFER_BYTES).is_err() {
            return Err(QueueSetupFailure::InvalidBuffer);
        }
        let cursor = self
            .cursors
            .get_mut(queue)
            .ok_or(QueueSetupFailure::InvalidRing {
                queue: queue as u32,
            })?;
        let ring = self.rings[queue];
        fence(Ordering::Acquire);
        // SAFETY: the ring was validated and retained by configure; the
        // device is the sole writer of used.idx.
        let device_used = unsafe { read_u16(ring.used_pa + 2) };
        if cursor.avail.wrapping_sub(device_used) >= QUEUE_SIZE as u16 {
            return Err(QueueSetupFailure::QueueFull {
                queue: queue as u32,
            });
        }
        let descriptor = cursor.avail & (QUEUE_SIZE as u16 - 1);
        let desc = ring.desc_pa + descriptor as u64 * DESC_BYTES as u64;
        // SAFETY: descriptor and available ring are EL1-owned pages retained
        // by this object; the bounds are fixed by QUEUE_SIZE.
        unsafe {
            write_u64(desc, buffer_pa);
            write_u32(desc + 8, len as u32);
            write_u16(desc + 12, flags);
            write_u16(desc + 14, 0);
            let avail_slot = cursor.avail as usize & (QUEUE_SIZE - 1);
            write_u16(
                ring.avail_pa + AVAIL_HEADER_BYTES as u64 + avail_slot as u64 * 2,
                descriptor,
            );
            fence(Ordering::Release);
            cursor.avail = cursor.avail.wrapping_add(1);
            write_u16(ring.avail_pa + 2, cursor.avail);
        }
        self.mmio.write32(virtio::mmio::QUEUE_NOTIFY, queue as u32);
        Ok(())
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
    let cursors = configure_queues(mmio, rings)?;

    Ok(Configured {
        mmio,
        negotiated,
        rings,
        cursors,
    })
}

fn configure_queues(
    mmio: Mmio,
    rings: [QueueMemory; QUEUE_COUNT],
) -> Result<[QueueCursor; QUEUE_COUNT], QueueSetupFailure> {
    for (queue, ring) in rings.iter().enumerate() {
        if ring.desc_pa == 0
            || ring.avail_pa == 0
            || ring.used_pa == 0
            || !ring.desc_pa.is_multiple_of(16)
            || !ring.avail_pa.is_multiple_of(2)
            || !ring.used_pa.is_multiple_of(4)
        {
            write_status(mmio, 0);
            return Err(QueueSetupFailure::InvalidRing {
                queue: queue as u32,
            });
        }
    }

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

    Ok([QueueCursor { avail: 0, used: 0 }; QUEUE_COUNT])
}

// The ring pages are identity mapped Normal memory in the EL1 address space.
// These helpers are private so no caller can turn the driver into a general
// physical-memory accessor.
unsafe fn read_u16(pa: u64) -> u16 {
    // SAFETY: callers prove the address belongs to a retained split-ring page.
    unsafe { core::ptr::read_volatile(pa as *const u16) }
}

unsafe fn read_u32(pa: u64) -> u32 {
    // SAFETY: callers prove the address belongs to a retained split-ring page.
    unsafe { core::ptr::read_volatile(pa as *const u32) }
}

unsafe fn write_u16(pa: u64, value: u16) {
    // SAFETY: callers prove the address belongs to a retained split-ring page.
    unsafe { core::ptr::write_volatile(pa as *mut u16, value) }
}

unsafe fn write_u32(pa: u64, value: u32) {
    // SAFETY: callers prove the address belongs to a retained split-ring page.
    unsafe { core::ptr::write_volatile(pa as *mut u32, value) }
}

unsafe fn write_u64(pa: u64, value: u64) {
    // SAFETY: callers prove the address belongs to a retained split-ring page.
    unsafe { core::ptr::write_volatile(pa as *mut u64, value) }
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
