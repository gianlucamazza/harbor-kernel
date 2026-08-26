//! QEMU virtio-net implementation of the board transport contract.
//!
//! The board-selected half of ADR-0112 §2 for `board-qemu-virt`: this module
//! owns the virtio-mmio slot scan, the split-queue descriptor plumbing and the
//! cache maintenance those rings need, behind the seven-operation
//! [`Transport`](crate::bsp::net::Transport) surface. Packet-pool policy,
//! frame ownership and generations stay above the boundary in
//! `network_runtime`.
//!
//! Direction of data: [`Transport::poll`] walks both used rings once, moving
//! finished descriptors into small ready queues (and compacting received
//! payloads to the front of their buffers);
//! [`take_rx_available`](crate::bsp::net::Transport::take_rx_available) and
//! [`take_tx_complete`](crate::bsp::net::Transport::take_tx_complete) pop those
//! queues. Nothing else consumes a used-ring entry, so the service observes a
//! completion exactly once.

use kernel_core::net::{self, PacketToken};

use super::memmap::{VIRTIO_MMIO_SLOTS, VIRTIO_MMIO_STRIDE, VIRTIO_NET_BASE};
use crate::arch::cache;
use crate::arch::mmio::Mmio;
use crate::bsp::net::{Buffer, Completion, RxSlot, StartFacts, Transport};
use crate::drivers::virtio_mmio::{self, Configured, QueueMemory, QueueSetupFailure};

/// The virtio-net wire header this backend stages ahead of every payload.
const VIRTIO_NET_HEADER_BYTES: usize = 12;
/// One ring region fits a page at the driver's fixed queue size.
const RING_PAGE_BYTES: usize = 4096;
/// Depth of one virtqueue; `Configured::queue_size` reports the same number.
const QUEUE_DEPTH: usize = 8;
const RX_QUEUE: usize = 0;
const TX_QUEUE: usize = 1;

pub type Backend = Virtio;

/// Whole pages this backend consumes for its two split-ring queues: one page
/// each for the descriptor, available and used regions, per queue.
pub const SCRATCH_PAGES: usize = 6;

const PROBE_PAYLOAD: &[u8] = b"harbor-p3-virtio-tx";

/// The QEMU virtio-net backend: one modern virtio-mmio slot with both split
/// queues bound. Selected by the board module; never a trait object.
pub struct Virtio {
    configured: Configured,
    rings: [QueueMemory; 2],
    /// Identities keyed by the descriptor position the device reports in its
    /// used ring (the avail-cursor value at post time, masked by the depth).
    rx_pending: [Option<(PacketToken, Buffer)>; QUEUE_DEPTH],
    tx_pending: [Option<PacketToken>; QUEUE_DEPTH],
    rx_cursor: u16,
    tx_cursor: u16,
    /// Completions `poll` found, waiting for the service to take them.
    rx_ready: [Option<(PacketToken, Completion)>; QUEUE_DEPTH],
    tx_ready: [Option<(PacketToken, Completion)>; QUEUE_DEPTH],
}

impl Drop for Virtio {
    fn drop(&mut self) {
        // Quiesce the device so a dropped lease leaves no live queues behind:
        // the frames these rings lived in are freed right after this drop.
        self.configured.reset();
    }
}

/// Scan the emulated virtio-mmio aperture and bind both split queues of the
/// first modern virtio-net device, using `scratch` pages as ring memory.
///
/// Safe because this board maps the whole aperture
/// (`VIRTIO_NET_BASE .. + VIRTIO_MMIO_STRIDE * VIRTIO_MMIO_SLOTS`) as Device
/// memory in its compiled region table, and `scratch` pages arrive from
/// bootstrap's frame pool: exclusively owned, zeroed, identity-mapped Normal
/// memory allocated for exactly this purpose.
#[allow(clippy::missing_panics_doc)] // the aperture always has ≥ 1 slot
pub fn claim(scratch: &[u64; SCRATCH_PAGES]) -> Result<(Virtio, StartFacts), QueueSetupFailure> {
    let rings = [
        QueueMemory {
            desc_pa: scratch[0],
            avail_pa: scratch[1],
            used_pa: scratch[2],
        },
        QueueMemory {
            desc_pa: scratch[3],
            avail_pa: scratch[4],
            used_pa: scratch[5],
        },
    ];
    let mut last_error = None;
    for slot in 0..VIRTIO_MMIO_SLOTS {
        let base = VIRTIO_NET_BASE + slot * VIRTIO_MMIO_STRIDE;
        // SAFETY: every QEMU virt slot lies in the aperture the compiled
        // region table maps Device-nGnRnE for this board.
        match unsafe { virtio_mmio::configure(Mmio::new(base), rings) } {
            Ok(configured) => {
                let facts = StartFacts {
                    base: base as u64,
                    identity: configured.negotiated().device.vendor,
                    features: configured.negotiated().features,
                    queues: configured.queue_count(),
                    queue_size: configured.queue_size(),
                };
                return Ok((
                    Virtio {
                        configured,
                        rings,
                        rx_pending: [None; QUEUE_DEPTH],
                        tx_pending: [None; QUEUE_DEPTH],
                        rx_cursor: 0,
                        tx_cursor: 0,
                        rx_ready: [None; QUEUE_DEPTH],
                        tx_ready: [None; QUEUE_DEPTH],
                    },
                    facts,
                ));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.expect("QEMU virt has at least one virtio-mmio slot"))
}

impl Virtio {
    /// Publish one receive buffer to the device and remember its identity.
    ///
    /// The descriptor covers the header room plus the payload capacity of the
    /// buffer page; the payload lands behind the device-written header.
    fn post_rx(&mut self, slot: RxSlot) -> Result<(), QueueSetupFailure> {
        let posted = VIRTIO_NET_HEADER_BYTES + slot.buffer.len;
        // SAFETY: the buffer is an EL1-owned packet page retained by the
        // lease; header room plus capacity stays within the 4 KiB page.
        unsafe {
            cache::clean_dcache_poc(slot.buffer.pa as usize, posted);
        }
        self.configured.post_rx(slot.buffer.pa, posted)?;
        let position = usize::from(self.rx_cursor & (QUEUE_DEPTH as u16 - 1));
        self.rx_pending[position] = Some((slot.token, slot.buffer));
        self.rx_cursor = self.rx_cursor.wrapping_add(1);
        Ok(())
    }

    /// Hand a receive buffer straight back after a refused frame.
    fn repost_rx(&mut self, token: PacketToken, buffer: Buffer) {
        let _ = self.return_rx(token, buffer);
    }

    /// Walk the receive used ring once, compacting payloads and queueing
    /// completions. Malformed frames go back to the device unreported.
    fn advance_rx(&mut self) -> Result<(), QueueSetupFailure> {
        loop {
            let Some(used) = self.configured.poll_used(RX_QUEUE)? else {
                return Ok(());
            };
            let position = usize::from(used.descriptor & (QUEUE_DEPTH as u16 - 1));
            let Some((token, buffer)) = self.rx_pending[position].take() else {
                continue;
            };
            if used.len < VIRTIO_NET_HEADER_BYTES as u32
                || used.len as usize > VIRTIO_NET_HEADER_BYTES + buffer.len
            {
                self.repost_rx(token, buffer);
                continue;
            }
            let payload = used.len as usize - VIRTIO_NET_HEADER_BYTES;
            if net::validate_frame_length(payload).is_err() {
                self.repost_rx(token, buffer);
                continue;
            }
            // SAFETY: the device owned this range until the used-ring entry
            // above; drop stale lines, then compact the payload to the front
            // of the buffer so no caller above the boundary ever sees a
            // device header.
            unsafe {
                cache::invalidate_dcache_poc(buffer.pa as usize, used.len as usize);
                core::ptr::copy(
                    (buffer.pa + VIRTIO_NET_HEADER_BYTES as u64) as *const u8,
                    buffer.pa as *mut u8,
                    payload,
                );
            }
            enqueue(
                &mut self.rx_ready,
                token,
                Completion {
                    buffer: used.descriptor,
                    len: payload,
                },
            );
        }
    }

    /// Walk the transmit used ring once, queueing completions.
    fn advance_tx(&mut self) -> Result<(), QueueSetupFailure> {
        while let Some(used) = self.configured.poll_used(TX_QUEUE)? {
            let position = usize::from(used.descriptor & (QUEUE_DEPTH as u16 - 1));
            let Some(token) = self.tx_pending[position].take() else {
                continue;
            };
            enqueue(
                &mut self.tx_ready,
                token,
                Completion {
                    buffer: used.descriptor,
                    len: used.len as usize,
                },
            );
        }
        Ok(())
    }
}

/// Park one completion in a ready queue. Both queues hold at most the device
/// depth, and every parked entry consumed one outstanding descriptor, so the
/// scan below always finds room.
fn enqueue(
    queue: &mut [Option<(PacketToken, Completion)>; QUEUE_DEPTH],
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

impl crate::bsp::net::Transport for Virtio {
    type Error = QueueSetupFailure;

    fn start(&mut self, rx: &[RxSlot], tx: Buffer) -> Result<(), Self::Error> {
        if rx.len() > QUEUE_DEPTH {
            // The device cannot hold this many outstanding receives; refusing
            // before the first post keeps `start` all-or-nothing.
            return Err(QueueSetupFailure::QueueFull {
                queue: RX_QUEUE as u32,
            });
        }
        publish_ring(&self.rings);
        for slot in rx {
            self.post_rx(*slot)?;
        }
        publish_ring(&self.rings);

        // Bring-up witness: one broadcast 0x88b5 frame so the link partner's
        // capture proves the transmit path before any agent exists. Authored
        // here because the virtio-net header shape is this backend's secret.
        // SAFETY: the probe page is an EL1-owned packet page retained by the
        // lease; header plus frame stays far inside the 4 KiB page.
        unsafe {
            let buffer = tx.pa as *mut u8;
            core::ptr::write_bytes(
                buffer.add(VIRTIO_NET_HEADER_BYTES),
                0,
                net::ETHERNET_MIN_FRAME_BYTES,
            );
            core::ptr::copy_nonoverlapping(
                [0xffu8; 6].as_ptr(),
                buffer.add(VIRTIO_NET_HEADER_BYTES),
                6,
            );
            buffer
                .add(VIRTIO_NET_HEADER_BYTES + 6)
                .copy_from_nonoverlapping([2, 0, 0, 0, 0, 1].as_ptr(), 6);
            *buffer.add(VIRTIO_NET_HEADER_BYTES + 12) = 0x88;
            *buffer.add(VIRTIO_NET_HEADER_BYTES + 13) = 0xb5;
            core::ptr::copy_nonoverlapping(
                PROBE_PAYLOAD.as_ptr(),
                buffer.add(VIRTIO_NET_HEADER_BYTES + 14),
                PROBE_PAYLOAD.len(),
            );
            cache::clean_dcache_poc(
                tx.pa as usize,
                VIRTIO_NET_HEADER_BYTES + net::ETHERNET_MIN_FRAME_BYTES,
            );
        }
        self.configured.submit_tx(
            tx.pa,
            VIRTIO_NET_HEADER_BYTES + net::ETHERNET_MIN_FRAME_BYTES,
        )?;
        let position = usize::from(self.tx_cursor & (QUEUE_DEPTH as u16 - 1));
        // The probe predates any pool identity; its completion carries the
        // service's bring-up slot so the existing transcript line survives.
        self.tx_pending[position] = Some(PacketToken {
            slot: 0,
            generation: 0,
            len: 0,
        });
        self.tx_cursor = self.tx_cursor.wrapping_add(1);
        publish_ring(&self.rings);
        Ok(())
    }

    fn submit_tx(&mut self, token: PacketToken, buffer: Buffer) -> Result<(), Self::Error> {
        net::validate_frame_length(buffer.len).map_err(|_| QueueSetupFailure::InvalidBuffer)?;
        // Stage the device framing: shift the payload behind a zeroed
        // virtio-net header inside the same page.
        // SAFETY: source and shifted destination stay within the one packet
        // page the lease owns; `core::ptr::copy` tolerates the overlap.
        unsafe {
            core::ptr::copy(
                buffer.pa as *const u8,
                (buffer.pa + VIRTIO_NET_HEADER_BYTES as u64) as *mut u8,
                buffer.len,
            );
            core::ptr::write_bytes(buffer.pa as *mut u8, 0, VIRTIO_NET_HEADER_BYTES);
            cache::clean_dcache_poc(buffer.pa as usize, VIRTIO_NET_HEADER_BYTES + buffer.len);
        }
        self.configured
            .submit_tx(buffer.pa, VIRTIO_NET_HEADER_BYTES + buffer.len)?;
        let position = usize::from(self.tx_cursor & (QUEUE_DEPTH as u16 - 1));
        self.tx_pending[position] = Some(token);
        self.tx_cursor = self.tx_cursor.wrapping_add(1);
        publish_ring(&self.rings);
        Ok(())
    }

    fn return_rx(&mut self, token: PacketToken, buffer: Buffer) -> Result<(), Self::Error> {
        self.post_rx(RxSlot { token, buffer })?;
        publish_ring(&self.rings);
        Ok(())
    }

    fn take_tx_complete(&mut self) -> Option<(PacketToken, Completion)> {
        let completion = self.tx_ready.iter_mut().find_map(|entry| entry.take());
        if let Some((token, completion)) = completion {
            if token.slot == 0 && token.generation == 0 && token.len == 0 {
                crate::kprintln!(
                    "virtio-net: tx descriptor complete used_len={}",
                    completion.len
                );
            }
            return Some((token, completion));
        }
        None
    }

    fn take_rx_available(&mut self) -> Option<(PacketToken, Completion)> {
        let completion = self.rx_ready.iter_mut().find_map(|entry| entry.take());
        if let Some((token, completion)) = completion {
            crate::kprintln!("virtio-net: rx available len={}", completion.len);
            return Some((token, completion));
        }
        None
    }

    fn poll(&mut self) -> Result<(), Self::Error> {
        consume_used(&self.rings);
        // The receive walk runs second so a burst of echoes cannot starve the
        // transmit side of a single pass.
        self.advance_tx()?;
        self.advance_rx()
    }

    fn reset(&mut self) -> Result<(), Self::Error> {
        self.configured.restart()?;
        self.rx_pending = [None; QUEUE_DEPTH];
        self.tx_pending = [None; QUEUE_DEPTH];
        self.rx_ready = [None; QUEUE_DEPTH];
        self.tx_ready = [None; QUEUE_DEPTH];
        self.rx_cursor = 0;
        self.tx_cursor = 0;
        // The runtime owns token generations. It reposts fresh RX identities
        // after resetting its pool, so this backend only restores the device
        // handshake and clears stale descriptor bookkeeping here.
        crate::kprintln!("virtio-net: recovery complete");
        Ok(())
    }
}

/// Make the driver-written ring regions visible to the device.
fn publish_ring(rings: &[QueueMemory; 2]) {
    for ring in rings {
        // SAFETY: these pages are the lease's exclusively-owned split rings.
        unsafe {
            cache::clean_dcache_poc(ring.desc_pa as usize, RING_PAGE_BYTES);
            cache::clean_dcache_poc(ring.avail_pa as usize, RING_PAGE_BYTES);
        }
    }
}

/// Drop stale lines over the device-written used rings before reading them.
fn consume_used(rings: &[QueueMemory; 2]) {
    for ring in rings {
        // SAFETY: the device owns the used-ring updates after publication.
        unsafe {
            cache::invalidate_dcache_poc(ring.used_pa as usize, RING_PAGE_BYTES);
        }
    }
}
