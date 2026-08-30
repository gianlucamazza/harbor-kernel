//! Resident transport ownership below the network service.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use kernel_core::frame::FrameId;
use kernel_core::net::{self as net_abi, PacketPool, PacketToken};

use crate::arch::cache;
use crate::bsp::net::{self as transport, Transport};
use crate::mm;
use crate::sync::Mutex;

const PACKET_PAGE_COUNT: usize = 8;
const DMA_PACKET_COUNT: usize = 9;
const SCRATCH_COUNT: usize = transport::SCRATCH_PAGES;

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
    Device(transport::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceError {
    Unavailable,
    Busy,
    Packet(kernel_core::net::PacketError),
    Transport(transport::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    Unavailable,
    TransportReset(transport::Error),
    PoolPublish { slot: u8 },
    PoolReturn { slot: u8 },
    TransportReturn { slot: u8, error: transport::Error },
}

#[derive(Clone, Copy)]
struct Page {
    id: FrameId,
    pa: usize,
}

struct Lease {
    transport: transport::Backend,
    scratch: [Page; SCRATCH_COUNT],
    packets: [Page; PACKET_PAGE_COUNT],
    dma_packets: [Page; DMA_PACKET_COUNT],
    pool: PacketPool,
    rx_slots: [u8; net_abi::PACKET_SLOTS / 2],
    service_tx: Option<PacketToken>,
    tx_event: Option<PacketToken>,
    rx_event: Option<PacketToken>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.transport.reset();
        free_pages(&self.scratch);
        free_pages(&self.packets);
        free_pages(&self.dma_packets);
    }
}

static LEASE: Mutex<Option<Lease>> = Mutex::new(None);
static RX_PACKETS: AtomicU32 = AtomicU32::new(0);
static TX_PACKETS: AtomicU32 = AtomicU32::new(0);
static REFUSED_PACKETS: AtomicU32 = AtomicU32::new(0);
static SERVICE_ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(any(feature = "board-qemu-virt", feature = "board-rpi4"))]
pub fn packet_pool_pages() -> Option<[usize; crate::mm::aspace::PACKET_POOL_PAGES]> {
    LEASE.with(|lease| {
        let lease = lease.as_ref()?;
        Some(core::array::from_fn(|i| lease.packets[i].pa))
    })
}

pub fn enable_service() {
    SERVICE_ACTIVE.store(true, Ordering::Release);
}

/// Whether the resident transport lease was claimed successfully.
pub fn service_available() -> bool {
    LEASE.with(|lease| lease.is_some())
}

pub fn start() -> Result<Report, StartError> {
    crate::kprintln!("net: transport start");
    if LEASE.with(|lease| lease.is_some()) {
        crate::kprintln!("net: transport start FAILED already started");
        return Err(StartError::AlreadyStarted);
    }
    let packets = match allocate_pages::<PACKET_PAGE_COUNT>() {
        Some(packets) => packets,
        None => {
            crate::kprintln!("net: packet pages FAILED");
            return Err(StartError::FramesUnavailable);
        }
    };
    let dma_packets = match allocate_pages::<DMA_PACKET_COUNT>() {
        Some(p) => p,
        None => {
            crate::kprintln!("net: DMA packet pages FAILED");
            free_pages(&packets);
            return Err(StartError::FramesUnavailable);
        }
    };
    let scratch = match allocate_pages::<SCRATCH_COUNT>() {
        Some(p) => p,
        None => {
            crate::kprintln!("net: scratch pages FAILED");
            free_pages(&packets);
            free_pages(&dma_packets);
            return Err(StartError::FramesUnavailable);
        }
    };
    let scratch_pa = core::array::from_fn(|i| scratch[i].pa as u64);
    let (mut backend, facts) = match transport::claim(&scratch_pa) {
        Ok(value) => value,
        Err(error) => {
            crate::kprintln!("net: transport claim FAILED {error:?}");
            free_pages(&scratch);
            free_pages(&packets);
            free_pages(&dma_packets);
            return Err(StartError::Device(error));
        }
    };
    let mut pool = PacketPool::new();
    let rx_slots = core::array::from_fn(|i| (net_abi::PACKET_SLOTS / 2 + i) as u8);
    let rx_set: [transport::RxSlot; net_abi::PACKET_SLOTS / 2] = core::array::from_fn(|i| {
        let token = pool
            .publish_rx(usize::from(rx_slots[i]), 0)
            .expect("fresh RX slot");
        pool.return_rx(token).expect("fresh RX return");
        transport::RxSlot {
            token,
            buffer: transport::Buffer {
                pa: dma_packets[i + 1].pa as u64,
                len: net_abi::PACKET_BYTES,
            },
        }
    });
    if let Err(error) = backend.start(
        &rx_set,
        transport::Buffer {
            pa: dma_packets[0].pa as u64,
            len: net_abi::PACKET_BYTES,
        },
    ) {
        let _ = backend.reset();
        crate::kprintln!("net: transport start FAILED {error:?}");
        free_pages(&scratch);
        free_pages(&packets);
        free_pages(&dma_packets);
        return Err(StartError::Device(error));
    }
    LEASE.with(|lease| {
        *lease = Some(Lease {
            transport: backend,
            scratch,
            packets,
            dma_packets,
            pool,
            rx_slots,
            service_tx: None,
            tx_event: None,
            rx_event: None,
        })
    });
    crate::kprintln!(
        "net: transport started base={:#x} queues={} size={}",
        facts.base,
        facts.queues,
        facts.queue_size
    );
    Ok(Report {
        base: facts.base as usize,
        vendor: facts.identity,
        features: facts.features,
        queues: facts.queues,
        queue_size: facts.queue_size,
        tx_submitted: true,
    })
}

pub fn submit_service_tx(token: PacketToken) -> Result<(), ServiceError> {
    LEASE.with(|lease| {
        let lease = lease.as_mut().ok_or(ServiceError::Unavailable)?;
        if lease.service_tx.is_some() {
            return Err(ServiceError::Busy);
        }
        lease.pool.accept_tx(token).map_err(ServiceError::Packet)?;
        let source = pool_address(&lease.packets, token.slot);
        // SAFETY: the pool validated `token`; both pages belong to this lease,
        // and the bounded length stays within the packet halves.
        unsafe {
            cache::clean_dcache_poc(source as usize, usize::from(token.len));
            core::ptr::copy_nonoverlapping(
                source as *const u8,
                lease.dma_packets[1].pa as *mut u8,
                usize::from(token.len),
            );
        }
        if let Err(error) = lease.transport.submit_tx(
            token,
            transport::Buffer {
                pa: lease.dma_packets[1].pa as u64,
                len: usize::from(token.len),
            },
        ) {
            // The pool transition above is speculative until the backend
            // accepts the descriptor. Restore agent ownership on refusal so a
            // transient link-down does not strand the slot forever.
            let _ = lease.pool.complete_tx(token);
            return Err(ServiceError::Transport(error));
        }
        lease.service_tx = Some(token);
        Ok(())
    })
}

pub fn return_service_rx(token: PacketToken) -> Result<(), ServiceError> {
    LEASE.with(|lease| {
        let lease = lease.as_mut().ok_or(ServiceError::Unavailable)?;
        lease.pool.return_rx(token).map_err(ServiceError::Packet)?;
        let i = usize::from(token.slot) - net_abi::PACKET_SLOTS / 2;
        lease
            .transport
            .return_rx(
                token,
                transport::Buffer {
                    pa: lease.dma_packets[i + 1].pa as u64,
                    len: net_abi::PACKET_BYTES,
                },
            )
            .map_err(ServiceError::Transport)
    })
}

pub fn take_tx_complete() -> Option<PacketToken> {
    LEASE.with(|lease| {
        let lease = lease.as_mut()?;
        if let Some(token) = lease.tx_event.take() {
            return Some(token);
        }
        let (token, _) = lease.transport.take_tx_complete()?;
        if lease.service_tx == Some(token) {
            lease.service_tx = None;
            let _ = lease.pool.complete_tx(token);
            lease.tx_event = Some(token);
        } else {
            TX_PACKETS.fetch_add(1, Ordering::Relaxed);
        }
        lease.tx_event.take()
    })
}

pub fn take_rx_available() -> Option<PacketToken> {
    LEASE.with(|lease| lease.as_mut()?.rx_event.take())
}

pub fn poll() {
    LEASE.with(|lease| {
        let Some(lease) = lease.as_mut() else { return };
        if lease.transport.poll().is_err() {
            REFUSED_PACKETS.fetch_add(1, Ordering::Relaxed);
            if let Err(error) = recover(lease) {
                crate::kprintln!("net: poll recovery FAILED {error:?}");
            }
            return;
        }
        while let Some((token, completion)) = lease.transport.take_rx_available() {
            let source = dma_address(&lease.dma_packets, token.slot);
            let destination = pool_address(&lease.packets, token.slot);
            // SAFETY: the transport returned a token previously posted by
            // this lease; completion.len is bounded by the packet buffer.
            unsafe {
                cache::invalidate_dcache_poc(source as usize, completion.len);
                core::ptr::copy_nonoverlapping(
                    source as *const u8,
                    destination as *mut u8,
                    completion.len,
                );
                cache::clean_dcache_poc(destination as usize, completion.len);
            }
            match lease
                .pool
                .publish_rx(usize::from(token.slot), completion.len)
            {
                Ok(published)
                    if SERVICE_ACTIVE.load(Ordering::Acquire) && lease.rx_event.is_none() =>
                {
                    lease.rx_event = Some(published);
                    RX_PACKETS.fetch_add(1, Ordering::Relaxed);
                }
                Ok(published) => {
                    let _ = lease.pool.return_rx(published);
                    let _ = lease.transport.return_rx(
                        published,
                        transport::Buffer {
                            pa: source,
                            len: net_abi::PACKET_BYTES,
                        },
                    );
                }
                Err(_) => {
                    REFUSED_PACKETS.fetch_add(1, Ordering::Relaxed);
                    let _ = lease.transport.return_rx(
                        token,
                        transport::Buffer {
                            pa: source,
                            len: net_abi::PACKET_BYTES,
                        },
                    );
                }
            }
        }
        while let Some((token, _)) = lease.transport.take_tx_complete() {
            if lease.service_tx == Some(token) {
                lease.service_tx = None;
                let _ = lease.pool.complete_tx(token);
                lease.tx_event = Some(token);
            } else {
                TX_PACKETS.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
}

pub fn recycle_after_session() -> Result<(), RecoveryError> {
    LEASE.with(|lease| {
        lease
            .as_mut()
            .ok_or(RecoveryError::Unavailable)
            .and_then(recover)
    })
}

fn recover(lease: &mut Lease) -> Result<(), RecoveryError> {
    lease
        .transport
        .reset()
        .map_err(RecoveryError::TransportReset)?;
    lease.pool.reset();
    lease.service_tx = None;
    lease.tx_event = None;
    lease.rx_event = None;
    for i in 0..lease.rx_slots.len() {
        let Ok(token) = lease.pool.publish_rx(usize::from(lease.rx_slots[i]), 0) else {
            return Err(RecoveryError::PoolPublish {
                slot: lease.rx_slots[i],
            });
        };
        lease
            .pool
            .return_rx(token)
            .map_err(|_| RecoveryError::PoolReturn {
                slot: lease.rx_slots[i],
            })?;
        lease
            .transport
            .return_rx(
                token,
                transport::Buffer {
                    pa: lease.dma_packets[i + 1].pa as u64,
                    len: net_abi::PACKET_BYTES,
                },
            )
            .map_err(|error| RecoveryError::TransportReturn {
                slot: lease.rx_slots[i],
                error,
            })?;
    }
    Ok(())
}

fn allocate_pages<const N: usize>() -> Option<[Page; N]> {
    let mut pages: [Option<Page>; N] = [None; N];
    for entry in &mut pages {
        let Some((id, pa)) = mm::frames::alloc() else {
            for page in pages.into_iter().flatten() {
                let _ = mm::frames::free(page.id);
            }
            return None;
        };
        // SAFETY: frame allocation returns an exclusive identity-mapped page;
        // zeroing it before publication establishes the DMA buffer contents.
        unsafe {
            core::ptr::write_bytes(pa as *mut u8, 0, 4096);
            cache::clean_dcache_poc(pa, 4096);
        }
        *entry = Some(Page { id, pa });
    }
    Some(core::array::from_fn(|i| {
        pages[i].unwrap_or(Page {
            id: FrameId::from_index(0),
            pa: 0,
        })
    }))
}

fn free_pages<const N: usize>(pages: &[Page; N]) {
    for page in pages {
        let _ = mm::frames::free(page.id);
    }
}

fn pool_address(pages: &[Page; PACKET_PAGE_COUNT], slot: u8) -> u64 {
    let slot = usize::from(slot);
    (pages[slot / 2].pa + (slot % 2) * net_abi::PACKET_BYTES) as u64
}

fn dma_address(pages: &[Page; DMA_PACKET_COUNT], slot: u8) -> u64 {
    pages[1 + usize::from(slot) - net_abi::PACKET_SLOTS / 2].pa as u64
}
