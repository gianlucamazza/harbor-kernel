//! Pure contracts for the P3 modern virtio-net service (ADR-0104).
//!
//! The EL1 driver owns MMIO, DMA memory, and descriptor rings.  This module
//! owns the arithmetic and ownership rules that must remain true regardless
//! of the transport implementation.  It deliberately has no MMIO, pointers,
//! or heap allocation, so malformed device data and agent messages are
//! testable on the host before the AArch64 driver exists.

/// `VIRTIO_F_VERSION_1`: the modern virtio transport contract.
pub const FEATURE_VERSION_1: u64 = 1 << 32;

/// The feature set the first P3 driver is prepared to negotiate.
pub const SUPPORTED_FEATURES: u64 = FEATURE_VERSION_1;

/// Why feature negotiation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureError {
    /// A device which does not expose the modern transport is not a P3 device.
    MissingModernTransport,
    /// The caller requested a feature outside the driver's supported set.
    UnsupportedRequired(u64),
}

/// Negotiate a feature set without silently falling back to legacy virtio.
///
/// `required` is the driver's non-negotiable contract. `optional` is masked
/// by [`SUPPORTED_FEATURES`], so a future caller cannot accidentally advertise
/// a feature whose handling has not been implemented.
pub fn negotiate_features(device: u64, required: u64, optional: u64) -> Result<u64, FeatureError> {
    if device & FEATURE_VERSION_1 == 0 {
        return Err(FeatureError::MissingModernTransport);
    }
    let unsupported = required & !SUPPORTED_FEATURES;
    if unsupported != 0 {
        return Err(FeatureError::UnsupportedRequired(unsupported));
    }
    if required & !device != 0 {
        return Err(FeatureError::UnsupportedRequired(required & !device));
    }
    Ok(required | (device & optional & SUPPORTED_FEATURES))
}

/// Split virtqueue descriptor size in bytes.
pub const DESC_BYTES: usize = 16;
/// Split virtqueue available-ring header size in bytes.
pub const AVAIL_HEADER_BYTES: usize = 4;
/// Split virtqueue used-ring header size in bytes.
pub const USED_HEADER_BYTES: usize = 4;
/// Alignment required by the used ring.
pub const USED_ALIGN: usize = 4;

/// Physical layout of a split virtqueue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitQueueLayout {
    pub desc_pa: u64,
    pub avail_pa: u64,
    pub used_pa: u64,
    pub bytes: usize,
}

/// Why split-queue arithmetic was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    ZeroSize,
    NotPowerOfTwo,
    UnalignedBase,
    AddressOverflow,
    SizeOverflow,
}

/// Calculate the modern split-ring layout using checked arithmetic.
///
/// The caller must provide a DMA-safe base and a queue size accepted by the
/// device.  This function only computes the layout; it does not grant the
/// resulting memory to EL0.
pub fn split_queue_layout(base: u64, size: usize) -> Result<SplitQueueLayout, QueueError> {
    if size == 0 {
        return Err(QueueError::ZeroSize);
    }
    if !size.is_power_of_two() {
        return Err(QueueError::NotPowerOfTwo);
    }
    if !base.is_multiple_of(DESC_BYTES as u64) {
        return Err(QueueError::UnalignedBase);
    }
    let desc_bytes = size
        .checked_mul(DESC_BYTES)
        .ok_or(QueueError::SizeOverflow)?;
    let avail_bytes = AVAIL_HEADER_BYTES
        .checked_add(size.checked_mul(2).ok_or(QueueError::SizeOverflow)?)
        .ok_or(QueueError::SizeOverflow)?;
    let used_bytes = USED_HEADER_BYTES
        .checked_add(size.checked_mul(8).ok_or(QueueError::SizeOverflow)?)
        .ok_or(QueueError::SizeOverflow)?;
    let avail_pa = base
        .checked_add(desc_bytes as u64)
        .ok_or(QueueError::AddressOverflow)?;
    let used_unaligned = avail_pa
        .checked_add(avail_bytes as u64)
        .ok_or(QueueError::AddressOverflow)?;
    let used_pa = used_unaligned
        .checked_add(USED_ALIGN as u64 - 1)
        .ok_or(QueueError::AddressOverflow)?
        & !(USED_ALIGN as u64 - 1);
    let end = used_pa
        .checked_add(used_bytes as u64)
        .ok_or(QueueError::AddressOverflow)?;
    let bytes = end
        .checked_sub(base)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(QueueError::SizeOverflow)?;
    Ok(SplitQueueLayout {
        desc_pa: base,
        avail_pa,
        used_pa,
        bytes,
    })
}

/// Fixed packet slot size mandated by ADR-0104.
pub const PACKET_BYTES: usize = 2 * 1024;
/// Bounded first-slice pool size.
pub const PACKET_SLOTS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotState {
    TxAgent,
    TxService,
    RxService,
    RxAgent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    generation: u32,
    state: SlotState,
}

/// A bounded TX/RX packet ownership table.
///
/// The first half is TX-owned by the agent and the second half is RX-owned by
/// the service.  Tokens contain only slot, generation, and length; no token
/// exposes a physical address or a kernel pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketPool {
    slots: [Slot; PACKET_SLOTS],
}

/// A packet reference carried across the service boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketToken {
    pub slot: u8,
    pub generation: u32,
    pub len: u16,
}

/// Why a packet operation was refused before touching device state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketError {
    SlotOutOfRange,
    StaleGeneration,
    WrongDirection,
    WrongOwner,
    Oversize,
}

impl PacketPool {
    /// Create a reset pool.  TX slots start with the agent; RX slots with EL1.
    pub const fn new() -> Self {
        let mut slots = [Slot {
            generation: 0,
            state: SlotState::TxAgent,
        }; PACKET_SLOTS];
        let mut i = PACKET_SLOTS / 2;
        while i < PACKET_SLOTS {
            slots[i].state = SlotState::RxService;
            i += 1;
        }
        Self { slots }
    }

    /// Submit a frame from an agent TX slot to the EL1 service.
    pub fn submit_tx(
        &mut self,
        slot: usize,
        generation: u32,
        len: usize,
    ) -> Result<PacketToken, PacketError> {
        let current = self.checked_slot(slot, generation)?;
        if slot >= PACKET_SLOTS / 2 {
            return Err(PacketError::WrongDirection);
        }
        if current.state != SlotState::TxAgent {
            return Err(PacketError::WrongOwner);
        }
        let len = u16::try_from(len).map_err(|_| PacketError::Oversize)?;
        if usize::from(len) > PACKET_BYTES {
            return Err(PacketError::Oversize);
        }
        self.slots[slot].state = SlotState::TxService;
        Ok(PacketToken {
            slot: slot as u8,
            generation,
            len,
        })
    }

    /// Return a completed TX slot to the agent.
    pub fn complete_tx(&mut self, token: PacketToken) -> Result<(), PacketError> {
        let slot = self.checked_token(token)?;
        if slot.state != SlotState::TxService {
            return Err(PacketError::WrongOwner);
        }
        self.slots[usize::from(token.slot)].state = SlotState::TxAgent;
        Ok(())
    }

    /// Publish an RX frame into an EL1-owned RX slot and notify the agent.
    pub fn publish_rx(&mut self, slot: usize, len: usize) -> Result<PacketToken, PacketError> {
        if slot >= PACKET_SLOTS {
            return Err(PacketError::SlotOutOfRange);
        }
        if slot < PACKET_SLOTS / 2 {
            return Err(PacketError::WrongDirection);
        }
        let current = self.slots[slot];
        if current.state != SlotState::RxService {
            return Err(PacketError::WrongOwner);
        }
        let len = u16::try_from(len).map_err(|_| PacketError::Oversize)?;
        if usize::from(len) > PACKET_BYTES {
            return Err(PacketError::Oversize);
        }
        self.slots[slot].state = SlotState::RxAgent;
        Ok(PacketToken {
            slot: slot as u8,
            generation: current.generation,
            len,
        })
    }

    /// Return an RX slot after the agent consumed it.
    pub fn return_rx(&mut self, token: PacketToken) -> Result<(), PacketError> {
        let slot = self.checked_token(token)?;
        if slot.state != SlotState::RxAgent {
            return Err(PacketError::WrongOwner);
        }
        self.slots[usize::from(token.slot)].state = SlotState::RxService;
        Ok(())
    }

    /// Reset the device contract and invalidate every outstanding token.
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            slot.generation = slot.generation.wrapping_add(1);
            slot.state = SlotState::TxAgent;
        }
        let mut i = PACKET_SLOTS / 2;
        while i < PACKET_SLOTS {
            self.slots[i].state = SlotState::RxService;
            i += 1;
        }
    }

    fn checked_slot(&self, slot: usize, generation: u32) -> Result<Slot, PacketError> {
        let current = *self.slots.get(slot).ok_or(PacketError::SlotOutOfRange)?;
        if current.generation != generation {
            return Err(PacketError::StaleGeneration);
        }
        Ok(current)
    }

    fn checked_token(&self, token: PacketToken) -> Result<Slot, PacketError> {
        if usize::from(token.len) > PACKET_BYTES {
            return Err(PacketError::Oversize);
        }
        self.checked_slot(usize::from(token.slot), token.generation)
    }
}

impl Default for PacketPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_negotiation_refuses_legacy_and_unknown_required_features() {
        assert_eq!(
            negotiate_features(0, FEATURE_VERSION_1, 0),
            Err(FeatureError::MissingModernTransport)
        );
        assert_eq!(
            negotiate_features(FEATURE_VERSION_1, 1 << 9, 0),
            Err(FeatureError::UnsupportedRequired(1 << 9))
        );
        assert_eq!(
            negotiate_features(FEATURE_VERSION_1, FEATURE_VERSION_1, u64::MAX),
            Ok(FEATURE_VERSION_1)
        );
    }

    #[test]
    fn split_layout_is_aligned_and_non_overlapping() {
        let q = split_queue_layout(0x1000, 8).unwrap();
        assert_eq!(q.desc_pa, 0x1000);
        assert_eq!(q.avail_pa, 0x1080);
        assert_eq!(q.used_pa, 0x1094);
        assert_eq!(q.bytes, 0x1094 + 4 + 8 * 8 - 0x1000);
    }

    #[test]
    fn split_layout_rejects_invalid_size_and_overflow() {
        assert_eq!(split_queue_layout(0, 0), Err(QueueError::ZeroSize));
        assert_eq!(split_queue_layout(0, 3), Err(QueueError::NotPowerOfTwo));
        assert_eq!(split_queue_layout(1, 2), Err(QueueError::UnalignedBase));
        assert_eq!(
            split_queue_layout(u64::MAX - 15, 2),
            Err(QueueError::AddressOverflow)
        );
    }

    #[test]
    fn tx_and_rx_ownership_is_directional() {
        let mut pool = PacketPool::new();
        let tx = pool.submit_tx(0, 0, 64).unwrap();
        assert_eq!(pool.submit_tx(0, 0, 64), Err(PacketError::WrongOwner));
        assert_eq!(pool.publish_rx(0, 64), Err(PacketError::WrongDirection));
        pool.complete_tx(tx).unwrap();
        let rx = pool.publish_rx(PACKET_SLOTS / 2, 128).unwrap();
        assert_eq!(pool.return_rx(rx), Ok(()));
        assert_eq!(pool.return_rx(rx), Err(PacketError::WrongOwner));
    }

    #[test]
    fn malformed_lengths_are_refused() {
        let mut pool = PacketPool::new();
        assert_eq!(
            pool.submit_tx(0, 0, PACKET_BYTES + 1),
            Err(PacketError::Oversize)
        );
        assert_eq!(
            pool.publish_rx(PACKET_SLOTS / 2, PACKET_BYTES + 1),
            Err(PacketError::Oversize)
        );
    }

    #[test]
    fn reset_invalidates_outstanding_tokens() {
        let mut pool = PacketPool::new();
        let tx = pool.submit_tx(0, 0, 1).unwrap();
        let rx = pool.publish_rx(PACKET_SLOTS / 2, 1).unwrap();
        pool.reset();
        assert_eq!(pool.complete_tx(tx), Err(PacketError::StaleGeneration));
        assert_eq!(pool.return_rx(rx), Err(PacketError::StaleGeneration));
        let fresh = pool.submit_tx(0, 1, 1).unwrap();
        assert_eq!(fresh.generation, 1);
    }
}
