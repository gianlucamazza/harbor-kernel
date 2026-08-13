//! Resident P3 transport ownership, below the future packet service.
//!
//! Bootstrap allocates the DMA pages because the driver layer must not import
//! the allocator. This module retains both the configured transport and the
//! frame ids until a later service lifecycle explicitly resets them.

use kernel_core::frame::FrameId;

use crate::arch::cache;
use crate::bsp::board;
use crate::drivers::virtio_mmio::{Configured, QueueMemory, QueueSetupFailure};
use crate::mm;
use crate::sync::Mutex;

const RING_PAGE_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub base: usize,
    pub vendor: u32,
    pub features: u64,
    pub queues: usize,
    pub queue_size: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartError {
    AlreadyStarted,
    FramesUnavailable,
    Device(QueueSetupFailure),
}

struct RingFrames {
    ids: [FrameId; 3],
    memory: QueueMemory,
}

impl RingFrames {
    fn release(&self) {
        for id in self.ids {
            let _ = mm::frames::free(id);
        }
    }
}

struct Lease {
    configured: Configured,
    rings: [RingFrames; 2],
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.configured.reset();
        for ring in &self.rings {
            ring.release();
        }
    }
}

static LEASE: Mutex<Option<Lease>> = Mutex::new(None);

/// Allocate rings and retain the configured EL1 transport for the resident
/// service. No agent capability is minted here.
pub fn start() -> Result<Report, StartError> {
    if LEASE.with(|lease| lease.is_some()) {
        return Err(StartError::AlreadyStarted);
    }
    let first = allocate_ring().ok_or(StartError::FramesUnavailable)?;
    let second = match allocate_ring() {
        Some(ring) => ring,
        None => {
            first.release();
            return Err(StartError::FramesUnavailable);
        }
    };
    let memory = [first.memory, second.memory];
    // SAFETY: bootstrap owns the only network lease; the BSP maps the window
    // as Device and the ring frames are identity-mapped Normal memory.
    let (report, configured) = match unsafe { board::network::configure(memory) } {
        Ok(result) => result,
        Err(error) => {
            first.release();
            second.release();
            return Err(StartError::Device(error));
        }
    };
    LEASE.with(|current| {
        *current = Some(Lease {
            configured,
            rings: [first, second],
        });
    });
    Ok(Report {
        base: report.base,
        vendor: report.negotiated.device.vendor,
        features: report.negotiated.features,
        queues: report.queues,
        queue_size: report.queue_size,
    })
}

fn allocate_ring() -> Option<RingFrames> {
    let first = mm::frames::alloc();
    let second = mm::frames::alloc();
    let third = mm::frames::alloc();
    let (Some((desc, desc_pa)), Some((avail, avail_pa)), Some((used, used_pa))) =
        (first, second, third)
    else {
        for allocation in [first, second, third].into_iter().flatten() {
            let _ = mm::frames::free(allocation.0);
        }
        return None;
    };
    for address in [desc_pa, avail_pa, used_pa] {
        // SAFETY: each frame is identity-mapped Normal RW and exclusively
        // owned by this ring until reset.
        unsafe {
            core::ptr::write_bytes(address as *mut u8, 0, RING_PAGE_BYTES);
            cache::clean_dcache_poc(address, RING_PAGE_BYTES);
        }
    }
    Some(RingFrames {
        ids: [desc, avail, used],
        memory: QueueMemory {
            desc_pa: desc_pa as u64,
            avail_pa: avail_pa as u64,
            used_pa: used_pa as u64,
        },
    })
}
