//! Resident P3 transport ownership, below the future packet service.
//!
//! Bootstrap allocates the DMA pages because the driver layer must not import
//! the allocator. This module retains both the configured transport and the
//! frame ids until a later service lifecycle explicitly resets them.

use core::sync::atomic::{AtomicU32, Ordering};

use kernel_core::frame::FrameId;
use kernel_core::virtio::{self, PacketPool};

use crate::arch::cache;
use crate::bsp::board;
use crate::drivers::virtio_mmio::{Configured, QueueMemory, QueueSetupFailure};
use crate::mm;
use crate::sync::Mutex;

const RING_PAGE_BYTES: usize = 4096;
const PACKET_PAGE_COUNT: usize = 8;
const DMA_PACKET_COUNT: usize = 9;
const PACKET_HEADER_BYTES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub base: usize,
    pub vendor: u32,
    pub features: u64,
    pub queues: usize,
    pub queue_size: usize,
    pub tx_submitted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartError {
    AlreadyStarted,
    FramesUnavailable,
    Device(QueueSetupFailure),
}

#[derive(Clone, Copy)]
struct PacketPage {
    id: FrameId,
    pa: usize,
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
    packets: [PacketPage; PACKET_PAGE_COUNT],
    dma_packets: [PacketPage; DMA_PACKET_COUNT],
    pool: PacketPool,
    rx_slots: [u8; 8],
    tx_token: Option<kernel_core::virtio::PacketToken>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.configured.reset();
        for ring in &self.rings {
            ring.release();
        }
        for page in self.packets {
            let _ = mm::frames::free(page.id);
        }
        for page in self.dma_packets {
            let _ = mm::frames::free(page.id);
        }
    }
}

static LEASE: Mutex<Option<Lease>> = Mutex::new(None);
static RX_PACKETS: AtomicU32 = AtomicU32::new(0);
static TX_PACKETS: AtomicU32 = AtomicU32::new(0);
static REFUSED_PACKETS: AtomicU32 = AtomicU32::new(0);

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
    let packets = match allocate_packets() {
        Some(packets) => packets,
        None => {
            first.release();
            second.release();
            return Err(StartError::FramesUnavailable);
        }
    };
    let dma_packets = match allocate_dma_packets() {
        Some(packets) => packets,
        None => {
            first.release();
            second.release();
            for page in packets {
                let _ = mm::frames::free(page.id);
            }
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
            for page in packets {
                let _ = mm::frames::free(page.id);
            }
            for page in dma_packets {
                let _ = mm::frames::free(page.id);
            }
            return Err(StartError::Device(error));
        }
    };
    let mut lease = Lease {
        configured,
        rings: [first, second],
        packets,
        dma_packets,
        pool: PacketPool::new(),
        rx_slots: core::array::from_fn(|i| (virtio::PACKET_SLOTS / 2 + i) as u8),
        tx_token: None,
    };
    for slot in 0..8 {
        if let Err(error) = lease.configured.post_rx(
            dma_address(&lease.dma_packets, 1 + slot),
            PACKET_HEADER_BYTES + virtio::PACKET_BYTES,
        ) {
            drop(lease);
            return Err(StartError::Device(error));
        }
    }
    publish_ring(&lease.rings[0], &lease.rings[1]);
    let tx_len = submit_probe_packet(&mut lease)?;
    let tx_token = lease
        .pool
        .submit_tx(0, 0, tx_len)
        .map_err(|_| StartError::Device(QueueSetupFailure::InvalidBuffer))?;
    lease.tx_token = Some(tx_token);
    if let Err(error) = lease.configured.submit_tx(
        dma_address(&lease.dma_packets, 0),
        PACKET_HEADER_BYTES + tx_len,
    ) {
        drop(lease);
        return Err(StartError::Device(error));
    }
    publish_ring(&lease.rings[0], &lease.rings[1]);
    LEASE.with(|current| *current = Some(lease));
    Ok(Report {
        base: report.base,
        vendor: report.negotiated.device.vendor,
        features: report.negotiated.features,
        queues: report.queues,
        queue_size: report.queue_size,
        tx_submitted: true,
    })
}

/// Poll completed device work from the voluntary EL1 path.
///
/// The IRQ handler only acknowledges the line. This function is called from
/// the idle loop, where it may take the service lock and perform bounded ring
/// work without violating the IRQ no-block/no-switch rule.
pub fn poll() {
    LEASE.with(|lease| {
        let Some(lease) = lease.as_mut() else { return };
        consume_used(&lease.rings[0], &lease.rings[1]);
        while let Ok(Some(used)) = lease.configured.poll_used(Configured::rx_queue()) {
            let descriptor = usize::from(used.descriptor);
            if descriptor >= lease.rx_slots.len() || used.len < PACKET_HEADER_BYTES as u32 {
                REFUSED_PACKETS.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let len = used.len as usize - PACKET_HEADER_BYTES;
            let slot = usize::from(lease.rx_slots[descriptor]);
            match lease.pool.publish_rx(slot, len) {
                Ok(_) => {
                    if RX_PACKETS.fetch_add(1, Ordering::Relaxed) == 0 {
                        crate::kprintln!("virtio-net: rx available len={}", len);
                    }
                    // The first slice has no EL0 consumer yet. Return the slot
                    // immediately only for the internal transport exercise;
                    // capability delivery is added with the real service.
                    let _ = lease.pool.return_rx(kernel_core::virtio::PacketToken {
                        slot: slot as u8,
                        generation: 0,
                        len: len as u16,
                    });
                    let _ = lease.configured.post_rx(
                        dma_address(&lease.dma_packets, 1 + descriptor),
                        PACKET_HEADER_BYTES + virtio::PACKET_BYTES,
                    );
                    publish_ring(&lease.rings[0], &lease.rings[1]);
                }
                Err(_) => {
                    REFUSED_PACKETS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        while let Ok(Some(used)) = lease.configured.poll_used(Configured::tx_queue()) {
            if used.len <= (PACKET_HEADER_BYTES + virtio::PACKET_BYTES) as u32 {
                if let Some(token) = lease.tx_token.take() {
                    let _ = lease.pool.complete_tx(token);
                }
                if TX_PACKETS.fetch_add(1, Ordering::Relaxed) == 0 {
                    crate::kprintln!("virtio-net: tx descriptor complete used_len={}", used.len);
                }
            } else {
                REFUSED_PACKETS.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}

fn publish_ring(first: &RingFrames, second: &RingFrames) {
    for ring in [first, second] {
        // SAFETY: these pages are the lease's exclusively-owned split rings.
        unsafe {
            cache::clean_dcache_poc(ring.memory.desc_pa as usize, RING_PAGE_BYTES);
            cache::clean_dcache_poc(ring.memory.avail_pa as usize, RING_PAGE_BYTES);
        }
    }
}

fn consume_used(first: &RingFrames, second: &RingFrames) {
    for ring in [first, second] {
        // SAFETY: the device owns the used-ring updates after publication.
        unsafe {
            cache::invalidate_dcache_poc(ring.memory.used_pa as usize, RING_PAGE_BYTES);
        }
    }
}

fn submit_probe_packet(lease: &mut Lease) -> Result<usize, StartError> {
    let payload = b"harbor-p3-virtio-tx";
    let frame_len = 14 + payload.len();
    let pa = dma_address(&lease.dma_packets, 0) as usize;
    // SAFETY: packet slot 0 is an EL1-owned, zeroed Normal buffer retained by
    // the lease; the writes remain within the 2 KiB slot.
    unsafe {
        let buffer = pa as *mut u8;
        core::ptr::write_bytes(buffer, 0, virtio::PACKET_BYTES);
        core::ptr::write_bytes(buffer.add(PACKET_HEADER_BYTES), 0xff, 6);
        buffer
            .add(PACKET_HEADER_BYTES + 6)
            .copy_from_nonoverlapping([2, 0, 0, 0, 0, 1].as_ptr(), 6);
        *buffer.add(PACKET_HEADER_BYTES + 12) = 0x88;
        *buffer.add(PACKET_HEADER_BYTES + 13) = 0xb5;
        core::ptr::copy_nonoverlapping(
            payload.as_ptr(),
            buffer.add(PACKET_HEADER_BYTES + 14),
            payload.len(),
        );
        cache::clean_dcache_poc(pa, PACKET_HEADER_BYTES + frame_len);
    }
    Ok(frame_len)
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

fn allocate_packets() -> Option<[PacketPage; PACKET_PAGE_COUNT]> {
    let mut pages = [None; PACKET_PAGE_COUNT];
    for page in &mut pages {
        *page = mm::frames::alloc().map(|(id, pa)| PacketPage { id, pa });
        if page.is_none() {
            for allocated in pages.into_iter().flatten() {
                let _ = mm::frames::free(allocated.id);
            }
            return None;
        }
        let packet = page.expect("packet page allocated");
        // SAFETY: the frame was just allocated and is identity-mapped Normal
        // memory; both 2 KiB packet halves must not expose a prior owner.
        unsafe {
            core::ptr::write_bytes(packet.pa as *mut u8, 0, RING_PAGE_BYTES);
            cache::clean_dcache_poc(packet.pa, RING_PAGE_BYTES);
        }
    }
    Some(core::array::from_fn(|i| {
        pages[i].expect("packet page allocated")
    }))
}

fn allocate_dma_packets() -> Option<[PacketPage; DMA_PACKET_COUNT]> {
    let mut pages = [None; DMA_PACKET_COUNT];
    for page in &mut pages {
        *page = mm::frames::alloc().map(|(id, pa)| PacketPage { id, pa });
        if page.is_none() {
            for allocated in pages.into_iter().flatten() {
                let _ = mm::frames::free(allocated.id);
            }
            return None;
        }
        let packet = page.expect("DMA packet page allocated");
        // SAFETY: the frame is exclusively owned by the EL1 transport and is
        // never mapped into an agent address space.
        unsafe {
            core::ptr::write_bytes(packet.pa as *mut u8, 0, RING_PAGE_BYTES);
            cache::clean_dcache_poc(packet.pa, RING_PAGE_BYTES);
        }
    }
    Some(core::array::from_fn(|i| {
        pages[i].expect("DMA packet page allocated")
    }))
}

fn dma_address(pages: &[PacketPage; DMA_PACKET_COUNT], slot: usize) -> u64 {
    pages[slot].pa as u64
}
