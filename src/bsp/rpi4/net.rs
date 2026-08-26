//! BCM2711 GENET implementation of the board transport contract.
//!
//! The board-selected half of ADR-0112 §3 for `board-rpi4`: queue-0 becomes a
//! real producer/consumer pair here, and the ring arithmetic is the pure model
//! ADR-0110 declared for exactly this slice — [`RingLayout`]/[`RingState`]/
//! [`RingError`] stop being design-ahead by being consumed. The bring-up
//! witness (`Genet::boot_after_program`) stays where it is; publication
//! programs its own rings beside it through the driver's service primitives.
//!
//! Polling only, like the virtio backend (ADR-0112 §4): binding `INTRL2` is a
//! separate slice with its own evidence. The only evidence for this backend's
//! datapath is silicon (ADR-0112 §5) — under QEMU `raspi4b` the FDT node is
//! deleted, `claim` refuses, and the vocabulary stays vacant, which is the
//! tested half of this file.

use kernel_core::genet::{
    self, BoundedRingState, Descriptor, DescriptorError, RingError, RingLayout, RingProgramError,
};
use kernel_core::net::{self, PacketToken};

use crate::arch::cache;
use crate::bsp::net::{Buffer, Completion, RxSlot, StartFacts, Transport};
use crate::drivers::genet::{Error, Genet};
use crate::sync::Mutex;

pub type Backend = GenetNet;

/// The rings live in GENET's descriptor RAM; no host pages are consumed.
// Keep the board-neutral bootstrap shape fixed. GENET owns its descriptor RAM
// in the device, so claim deliberately ignores these six scratch addresses.
pub const SCRATCH_PAGES: usize = 6;

/// Receive buffers published to the packet pool. The pool holds eight RX
/// slots, so eight outstanding receives cover it exactly.
const RX_SLOTS: usize = kernel_core::net::PACKET_SLOTS / 2;
/// The service publishes at most eight descriptors at once. The model keeps
/// that outstanding bound compact while its cursor still wraps at hardware's
/// full queue-0 ring count.
const OUTSTANDING: usize = RX_SLOTS;
/// Hardware span of queue 0: TDMA carries 128 BDs, RDMA all 256. The model
/// wraps where the hardware wraps, not where the service stops posting.
const TX_RING_COUNT: u16 = genet::V5_Q0_TX_BD_CNT;
const RX_RING_COUNT: u16 = genet::V5_Q0_RX_BD_CNT;
/// TBUF/RBUF run in 64-byte status-block mode: every transferred frame is
/// prefixed by a TSB, in TX descriptors ahead of the frame and in RX buffers
/// ahead of the received bytes.
const TSB: usize = genet::TSB_BYTES as usize;
const PROBE_PAYLOAD: &[u8] = b"harbor-p3-genet-tx";
/// The witness frame padded to the minimum Ethernet size, TSB excluded.
const PROBE_FRAME_BYTES: usize = genet::MIN_FRAME_BYTES as usize;

const TDMA: u32 = genet::registers::TDMA;
const RDMA: u32 = genet::registers::RDMA;

/// The probed controller between boot-time bring-up and service claim.
///
/// Bootstrap probes at discover time and runs the unpublished witness; the
/// resident network service takes the controller out of here when it starts.
/// One static, one owner at a time — a second handle to the same registers
/// would violate the `Mmio` exclusivity contract.
static CONTROLLER: Mutex<Option<Genet>> = Mutex::new(None);

/// Park the probed controller for the resident service to claim later.
pub fn offer(controller: Genet) {
    CONTROLLER.with(|held| *held = Some(controller));
}

/// Borrow the parked controller, if the probe found one.
///
/// Bring-up (`report_genet_queue0`) borrows without taking; only
/// [`claim`] consumes it.
pub fn with_controller<R>(f: impl FnOnce(Option<&mut Genet>) -> R) -> R {
    CONTROLLER.with(|held| f(held.as_mut()))
}

/// Take the probed controller and expose the service facts.
///
/// Refuses with [`Error::NotPresent`] when no controller was ever probed —
/// a NIC-less board, whose vocabulary stays vacant downstream.
pub fn claim(_scratch: &[u64; SCRATCH_PAGES]) -> Result<(GenetNet, StartFacts), Error> {
    let Some(controller) = CONTROLLER.with(|held| held.take()) else {
        return Err(Error::NotPresent);
    };
    let binding = controller.binding();
    let revision = controller.revision();
    let facts = StartFacts {
        base: binding.mmio_base,
        identity: (u32::from(revision.major) << 24)
            | (u32::from(revision.minor) << 16)
            | u32::from(revision.patch),
        features: 0,
        queues: 1,
        queue_size: RX_SLOTS,
    };
    Ok((GenetNet::new(controller)?, facts))
}

fn tx_layout() -> Result<RingLayout, Error> {
    RingLayout::new(TDMA as u64, TX_RING_COUNT).ok_or(Error::InvalidBinding)
}

fn rx_layout() -> Result<RingLayout, Error> {
    RingLayout::new(RDMA as u64, RX_RING_COUNT).ok_or(Error::InvalidBinding)
}

fn ring_error(error: RingError) -> Error {
    match error {
        RingError::InvalidStatus(error) | RingError::InvalidDescriptor(error) => {
            Error::Descriptor(error)
        }
        // Posting into a full model ring means more outstanding descriptors
        // than the hardware span; the bring-up vocabulary calls that TooMany.
        RingError::Full => Error::Ring(RingProgramError::TooMany),
        // Only reachable when the caller ignored the consumer index, which
        // the poll loop never does; a timeout is the honest shape anyway.
        RingError::NoCompletion => Error::Timeout,
    }
}

/// Park one completion in a ready queue. Outstanding work is bounded by the
/// published receive set and single-flight transmit, so the scan below always
/// finds room.
fn enqueue(
    queue: &mut [Option<(PacketToken, Completion)>],
    token: PacketToken,
    completion: Completion,
) {
    for entry in queue.iter_mut() {
        if entry.is_none() {
            *entry = Some((token, completion));
            return;
        }
    }
}

/// The published GENET backend: one probed controller plus two model-driven
/// rings over its queue-0 descriptor RAM.
pub struct GenetNet {
    controller: Genet,
    tx_ring: BoundedRingState<OUTSTANDING>,
    rx_ring: BoundedRingState<OUTSTANDING>,
    tx_tokens: [Option<PacketToken>; TX_RING_COUNT as usize],
    /// Receive identity and buffer address keyed by descriptor index; the
    /// address is needed again to invalidate and compact on delivery.
    rx_buffers: [Option<(PacketToken, u64)>; RX_RING_COUNT as usize],
    rx_ready: [Option<(PacketToken, Completion)>; RX_SLOTS],
    tx_ready: [Option<(PacketToken, Completion)>; RX_SLOTS],
    dataplane: DataplaneState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DataplaneState {
    Fresh,
    LinkUp,
    Programmed,
    Enabled,
}

impl Drop for GenetNet {
    fn drop(&mut self) {
        // Quiesce on teardown: stop both engines and latch UniMAC reset, the
        // same baseline `probe` establishes, so a dropped lease leaves no live
        // DMA behind.
        let _ = self.controller.reset();
    }
}

impl GenetNet {
    fn new(controller: Genet) -> Result<Self, Error> {
        let dma = controller.binding().dma;
        Ok(Self {
            tx_ring: BoundedRingState::new(tx_layout()?, dma),
            rx_ring: BoundedRingState::new(rx_layout()?, dma),
            controller,
            tx_tokens: [None; TX_RING_COUNT as usize],
            rx_buffers: [None; RX_RING_COUNT as usize],
            rx_ready: [None; RX_SLOTS],
            tx_ready: [None; RX_SLOTS],
            dataplane: DataplaneState::Fresh,
        })
    }

    fn translate(&self, cpu: u64, len: u64) -> Result<u64, Error> {
        self.controller
            .binding()
            .dma
            .map_cpu(cpu, len)
            .map_err(|_| Error::Descriptor(DescriptorError::AddressOutsideDma))
    }

    /// Linux `bcmgenet_open`'s controller configuration before descriptors are
    /// published.  Queue and MAC enable are intentionally separate: the DMA
    /// engines must not run until RX ownership is visible in the ring.
    fn program_dataplane(&mut self) -> Result<(), Error> {
        self.controller.program_umac_init();
        self.controller.program_tbuf_tsb();
        self.controller.program_rbuf_tbuf_size();
        self.controller.program_rbuf_64b();
        self.controller.program_rbuf_chk();
        self.controller.program_rgmii_oob();
        self.controller.clear_hfb();
        self.controller.flush_before_rings();
        self.controller.program_service_rings()?;
        self.controller.program_priority_tx_rings();
        self.controller.program_tdma_wrr();
        self.controller.program_tdma_priority();
        Ok(())
    }

    fn ensure_dataplane_programmed(&mut self) -> Result<(), Error> {
        if matches!(
            self.dataplane,
            DataplaneState::Programmed | DataplaneState::Enabled
        ) {
            return Ok(());
        }
        if self.dataplane != DataplaneState::Fresh {
            return Err(Error::Enable(
                kernel_core::genet::QueueEnableError::NotProgrammed,
            ));
        }
        crate::kprintln!("genet: link acquisition start window_us=5000000");
        self.controller.acquire_link()?;
        self.dataplane = DataplaneState::LinkUp;
        crate::kprintln!("genet: link acquisition up");
        self.program_dataplane()?;
        self.dataplane = DataplaneState::Programmed;
        let state = self.controller.read_service_snapshot(0);
        crate::kprintln!(
            "genet: dataplane programmed tdma={:#x}/{:#x} rdma={:#x}/{:#x} txbd={:#x} rxbd={:#x}",
            state.state.tdma_ctrl,
            state.state.tdma_status,
            state.state.rdma_ctrl,
            state.state.rdma_status,
            state.tx_desc_status,
            state.rx_desc_status
        );
        Ok(())
    }

    fn enable_dataplane(&mut self) -> Result<(), Error> {
        if self.dataplane == DataplaneState::Enabled {
            return Ok(());
        }
        if self.dataplane != DataplaneState::Programmed {
            return Err(Error::Enable(
                kernel_core::genet::QueueEnableError::NotProgrammed,
            ));
        }
        self.controller.enable_queue0()?;
        self.controller.enable_datapath()?;
        self.dataplane = DataplaneState::Enabled;
        let state = self.controller.read_service_snapshot(0);
        crate::kprintln!(
            "genet: dataplane enabled cmd={:#x} tdma={:#x}/{:#x} rdma={:#x}/{:#x} tx={}/{} rx={}/{}",
            state.state.umac_cmd,
            state.state.tdma_ctrl,
            state.state.tdma_status,
            state.state.rdma_ctrl,
            state.state.rdma_status,
            state.tx_prod,
            state.tx_cons,
            state.rx_prod,
            state.rx_cons
        );
        Ok(())
    }

    /// Post one receive buffer: translate, model-post, write the address-only
    /// descriptor, and remember the pairing.
    ///
    /// Before the device may write, stale CPU lines over the slot are dropped:
    /// the buffer is Normal-WB and the engine owns the range until delivery.
    fn post_rx_slot(&mut self, slot: RxSlot) -> Result<(), Error> {
        // SAFETY: the slot is an EL1-owned packet page retained by the lease;
        // TSB plus the ring slot size stays within the 4 KiB page.
        unsafe {
            cache::invalidate_dcache_poc(slot.buffer.pa as usize, TSB + slot.buffer.len);
        }
        let address = self.translate(slot.buffer.pa, u64::from(genet::RX_BUF_LENGTH))?;
        let descriptor = Descriptor {
            address,
            length: u32::from(genet::RX_BUF_LENGTH),
            status: 0,
        };
        let index = self.rx_ring.post(descriptor).map_err(ring_error)?;
        self.controller.write_descriptor(RDMA, index, descriptor)?;
        self.rx_buffers[usize::from(index)] = Some((slot.token, slot.buffer.pa));
        Ok(())
    }

    /// Author the device framing for one outgoing buffer and post it.
    ///
    /// The TSB is written ahead of the payload inside the same page, which is
    /// why the payload shifts up before transmission.
    fn submit_frame(&mut self, token: PacketToken, buffer: Buffer) -> Result<(), Error> {
        net::validate_frame_length(buffer.len).map_err(Error::Frame)?;
        // SAFETY: source and shifted destination stay within the one packet
        // page the lease owns; `core::ptr::copy` tolerates the overlap.
        unsafe {
            core::ptr::copy(
                buffer.pa as *const u8,
                (buffer.pa + TSB as u64) as *mut u8,
                buffer.len,
            );
            core::ptr::write_bytes(buffer.pa as *mut u8, 0, TSB);
            cache::clean_dcache_poc(buffer.pa as usize, TSB + buffer.len);
        }
        let total = (TSB + buffer.len) as u32;
        let address = self.translate(buffer.pa, u64::from(total))?;
        let descriptor = Descriptor {
            address,
            length: total,
            status: 0,
        };
        let index = self.tx_ring.post(descriptor).map_err(ring_error)?;
        self.controller.write_descriptor(TDMA, index, descriptor)?;
        self.tx_tokens[usize::from(index)] = Some(token);
        let state = self.controller.read_service_snapshot(index);
        // SAFETY: the caller retains this packet page and `TSB + 14` is
        // inside the validated Ethernet buffer.
        let frame_head = unsafe {
            (
                core::ptr::read_volatile((buffer.pa + TSB as u64) as *const u8),
                core::ptr::read_volatile((buffer.pa + TSB as u64 + 1) as *const u8),
                core::ptr::read_volatile((buffer.pa + TSB as u64 + 12) as *const u8),
                core::ptr::read_volatile((buffer.pa + TSB as u64 + 13) as *const u8),
            )
        };
        crate::kprintln!(
            "genet: tx prepared index={} len={} prod={}/{} desc={:#x} dma={:#x}:{:#x} frame={:#x}{:#x}{:#x}{:#x}",
            index,
            buffer.len,
            state.tx_prod,
            state.tx_cons,
            state.tx_desc_status,
            state.tx_desc_addr_hi,
            state.tx_desc_addr_lo,
            frame_head.0,
            frame_head.1,
            frame_head.2,
            frame_head.3
        );
        self.controller
            .doorbell_producer(TDMA, self.tx_ring.producer());
        let state = self.controller.read_service_snapshot(index);
        crate::kprintln!(
            "genet: tx doorbell prod={}/{} cmd={:#x} tdma={:#x}/{:#x}",
            state.tx_prod,
            state.tx_cons,
            state.state.umac_cmd,
            state.state.tdma_ctrl,
            state.state.tdma_status
        );
        Ok(())
    }

    /// Retire everything the engines finished since the last pass.
    fn advance_rings(&mut self) -> Result<(), Error> {
        let tx_cons = self.controller.consumer_index(TDMA) as u16;
        while self.tx_ring.consumer() != tx_cons {
            let index = self.tx_ring.consumer();
            let status = self.controller.descriptor_status(TDMA, index);
            match self.tx_ring.complete(status) {
                Ok((retired, descriptor)) => {
                    // TX words stay as posted (the silicon never writes OWN
                    // back), so the completion reports the transmitted length.
                    if let Some(token) = self.tx_tokens[usize::from(retired)].take() {
                        enqueue(
                            &mut self.tx_ready,
                            token,
                            Completion {
                                buffer: retired,
                                len: descriptor.length as usize,
                            },
                        );
                    }
                }
                Err(RingError::NoCompletion) => break,
                Err(error) => return Err(ring_error(error)),
            }
        }
        let rx_cons = self.controller.consumer_index(RDMA) as u16;
        while self.rx_ring.consumer() != rx_cons {
            let index = self.rx_ring.consumer();
            let status = self.controller.descriptor_status(RDMA, index);
            match self.rx_ring.complete(status) {
                Ok((retired, descriptor)) => {
                    let Some((token, pa)) = self.rx_buffers[usize::from(retired)].take() else {
                        continue;
                    };
                    let wire = descriptor.length as usize;
                    if wire < TSB || net::validate_frame_length(wire - TSB).is_err() {
                        // Outside the published contract: hand the slot back
                        // rather than report a frame the service would refuse.
                        let buffer = Buffer {
                            pa,
                            len: kernel_core::net::PACKET_BYTES,
                        };
                        let _ = self.return_rx(token, buffer);
                        continue;
                    }
                    let payload = wire - TSB;
                    // SAFETY: the engine owned this range until the consumer
                    // index moved above; drop stale lines, then compact the
                    // payload to the front of the buffer so no caller above
                    // the boundary ever sees the TSB prefix.
                    unsafe {
                        cache::invalidate_dcache_poc(pa as usize, wire);
                        core::ptr::copy((pa + TSB as u64) as *const u8, pa as *mut u8, payload);
                    }
                    // SAFETY: the device ownership ended when the consumer
                    // index advanced and the payload was compacted in place.
                    let (src0, src1, src2, src3, ether_hi, ether_lo, magic0, magic1) = unsafe {
                        (
                            core::ptr::read_volatile((pa + 6) as *const u8),
                            core::ptr::read_volatile((pa + 7) as *const u8),
                            core::ptr::read_volatile((pa + 8) as *const u8),
                            core::ptr::read_volatile((pa + 9) as *const u8),
                            core::ptr::read_volatile((pa + 12) as *const u8),
                            core::ptr::read_volatile((pa + 13) as *const u8),
                            core::ptr::read_volatile((pa + 14) as *const u8),
                            core::ptr::read_volatile((pa + 15) as *const u8),
                        )
                    };
                    crate::kprintln!(
                        "genet: rx frame len={} src={:#x}{:#x}{:#x}{:#x} ether={:#x}{:#x} magic={:#x}{:#x}",
                        payload,
                        src0,
                        src1,
                        src2,
                        src3,
                        ether_hi,
                        ether_lo,
                        magic0,
                        magic1
                    );
                    enqueue(
                        &mut self.rx_ready,
                        token,
                        Completion {
                            buffer: retired,
                            len: payload,
                        },
                    );
                }
                Err(RingError::NoCompletion) => break,
                Err(error) => return Err(ring_error(error)),
            }
        }
        Ok(())
    }
}

/// Build the broadcast probe's Ethernet frame into `buffer`, without a TSB.
/// `submit_frame` owns the single TSB insertion for every resident TX.
fn author_probe(buffer: Buffer) -> usize {
    // SAFETY: the probe page is an EL1-owned packet page retained by the
    // lease; the minimum Ethernet frame stays inside the packet page.
    unsafe {
        let page = core::slice::from_raw_parts_mut(buffer.pa as *mut u8, buffer.len);
        author_probe_frame(page);
    }
    PROBE_FRAME_BYTES
}

fn author_probe_frame(page: &mut [u8]) {
    debug_assert!(page.len() >= PROBE_FRAME_BYTES);
    page[..PROBE_FRAME_BYTES].fill(0);
    page[..6].fill(0xff);
    page[6..12].copy_from_slice(&genet::STATION_ADDR);
    page[12] = 0x88;
    page[13] = 0xb5;
    page[14..14 + PROBE_PAYLOAD.len()].copy_from_slice(PROBE_PAYLOAD);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_frame_has_one_ethernet_header_before_tsb_insertion() {
        let mut page = [0u8; PROBE_FRAME_BYTES];
        author_probe_frame(&mut page);

        assert_eq!(&page[..6], &[0xff; 6]);
        assert_eq!(&page[6..12], &genet::STATION_ADDR);
        assert_eq!(&page[12..14], &[0x88, 0xb5]);
        assert_eq!(&page[14..14 + PROBE_PAYLOAD.len()], PROBE_PAYLOAD);
        assert_eq!(page.len(), PROBE_FRAME_BYTES);
    }
}

impl crate::bsp::net::Transport for GenetNet {
    type Error = Error;

    fn start(&mut self, rx: &[RxSlot], tx: Buffer) -> Result<(), Self::Error> {
        crate::kprintln!("genet: service start rx_slots={}", rx.len());
        if rx.len() > RX_SLOTS {
            return Err(Error::Ring(RingProgramError::TooMany));
        }
        self.ensure_dataplane_programmed()?;
        for slot in rx {
            self.post_rx_slot(*slot)?;
        }
        self.controller
            .doorbell_producer(RDMA, self.rx_ring.producer());
        self.enable_dataplane()?;

        // Bring-up witness, published form: the same broadcast 0x88b5 proof
        // the accepted gate used, sent through the ring instead of descriptor
        // zero. Authored here because the TSB layout is this backend's secret.
        let probe_len = author_probe(tx);
        let descriptor_token = PacketToken {
            slot: 0,
            generation: 0,
            len: 0,
        };
        self.submit_frame(
            descriptor_token,
            Buffer {
                pa: tx.pa,
                len: probe_len,
            },
        )
    }

    fn submit_tx(&mut self, token: PacketToken, buffer: Buffer) -> Result<(), Self::Error> {
        if self.dataplane != DataplaneState::Enabled {
            return Err(Error::Enable(
                kernel_core::genet::QueueEnableError::NotProgrammed,
            ));
        }
        self.submit_frame(token, buffer)
    }

    fn return_rx(&mut self, token: PacketToken, buffer: Buffer) -> Result<(), Self::Error> {
        self.ensure_dataplane_programmed()?;
        self.post_rx_slot(RxSlot { token, buffer })?;
        self.enable_dataplane()?;
        self.controller
            .doorbell_producer(RDMA, self.rx_ring.producer());
        Ok(())
    }

    fn take_tx_complete(&mut self) -> Option<(PacketToken, Completion)> {
        let completion = self.tx_ready.iter_mut().find_map(|entry| entry.take());
        if let Some((token, completion)) = completion {
            if token.slot == 0 && token.generation == 0 && token.len == 0 {
                crate::kprintln!("genet: tx descriptor complete used_len={}", completion.len);
                let tsv = self.controller.settle_umac_tsv();
                crate::kprintln!(
                    "genet: umac tsv probe packed={} linux={} pok={} (mib, not a nic)",
                    tsv.packed,
                    tsv.linux,
                    tsv.pok
                );
            } else {
                let tsv = self.controller.settle_umac_tsv();
                crate::kprintln!(
                    "genet: umac tsv tx slot={} generation={} packed={} linux={} pok={} (mib, not a nic)",
                    token.slot,
                    token.generation,
                    tsv.packed,
                    tsv.linux,
                    tsv.pok
                );
            }
            return Some((token, completion));
        }
        None
    }

    fn take_rx_available(&mut self) -> Option<(PacketToken, Completion)> {
        let completion = self.rx_ready.iter_mut().find_map(|entry| entry.take());
        if let Some((token, completion)) = completion {
            crate::kprintln!("genet: rx available len={}", completion.len);
            return Some((token, completion));
        }
        None
    }

    fn poll(&mut self) -> Result<(), Self::Error> {
        self.advance_rings()
    }

    fn reset(&mut self) -> Result<(), Self::Error> {
        self.controller.reset()?;
        self.tx_ring = BoundedRingState::new(tx_layout()?, self.controller.binding().dma);
        self.rx_ring = BoundedRingState::new(rx_layout()?, self.controller.binding().dma);
        self.tx_tokens = [None; TX_RING_COUNT as usize];
        self.rx_buffers = [None; RX_RING_COUNT as usize];
        self.rx_ready = [None; RX_SLOTS];
        self.tx_ready = [None; RX_SLOTS];
        self.dataplane = DataplaneState::Fresh;
        // RX token ownership belongs to `network_runtime::recover`: it resets
        // the packet pool and publishes fresh generations after this device
        // reset. Re-posting the previous identities here would duplicate the
        // bounded RX ring and reintroduce stale tokens.
        crate::kprintln!("genet: recovery complete");
        Ok(())
    }
}
