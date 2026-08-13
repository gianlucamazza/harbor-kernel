//! Pure BCM2711 GENET v5 contracts (ADR-0106).
//!
//! This module deliberately stops at arithmetic and ownership. It does not
//! select a board address, touch MMIO, or expose a descriptor to EL0. The
//! eventual Pi 4 binding must supply a verified device-tree translation and
//! use these checks before programming the controller.

/// BCM2711 GENET v5 register window size from the device tree.
pub const REGISTER_BYTES: u64 = 0x1_0000;
/// One GENET descriptor's status/address words for v4 and later.
pub const DESCRIPTOR_BYTES: u64 = 12;
/// The hardware's total descriptor count, shared by TX and RX rings.
pub const TOTAL_DESCRIPTORS: u16 = 256;
/// Maximum standard Ethernet frame accepted by the first bounded slice.
pub const MAX_FRAME_BYTES: u32 = 1536;
/// GENET v5 DMA burst value required by BCM2711 platform data.
pub const BCM2711_DMA_BURST: u32 = 0x08;
pub const DMA_LENGTH_MASK: u32 = 0x0fff;
pub const DMA_LENGTH_SHIFT: u32 = 16;
pub const DMA_OWN: u32 = 0x8000;
pub const DMA_EOP: u32 = 0x4000;
pub const DMA_SOP: u32 = 0x2000;
pub const DMA_WRAP: u32 = 0x1000;

/// GENET register offsets used by the first bounded model.
pub mod registers {
    pub const SYS_REV_CTRL: u32 = 0x0000;
    pub const INTRL2_0: u32 = 0x0200;
    pub const INTRL2_1: u32 = 0x0240;
    pub const RBUF: u32 = 0x0300;
    pub const UMAC: u32 = 0x0800;
    pub const MDIO: u32 = 0x0e14;
    pub const RDMA: u32 = 0x2000;
    pub const TDMA: u32 = 0x4000;
}

/// GENET interrupt bits from the two INTRL2 instances.
pub mod interrupt {
    pub const LINK_EVENT: u32 = (1 << 4) | (1 << 5);
    pub const MDIO_EVENT: u32 = (1 << 23) | (1 << 24);
    pub const RX_DONE: u32 = 1 << 13;
    pub const TX_DONE: u32 = 1 << 16;
    pub const QUEUE_RX_SHIFT: u32 = 16;
    pub const QUEUE_TX_MASK: u32 = 0xffff;
}

/// A device-tree-derived DMA aperture. Addresses are inclusive at the lower
/// edge and exclusive at `end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaWindow {
    pub base: u64,
    pub len: u64,
}

/// The bounded set of DMA apertures supplied by a device-tree binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaWindows {
    pub windows: [DmaWindow; 4],
    pub count: u8,
}

impl DmaWindows {
    pub const fn new(windows: [DmaWindow; 4], count: u8) -> Option<Self> {
        if count == 0 || count > windows.len() as u8 {
            None
        } else {
            Some(Self { windows, count })
        }
    }

    pub const fn contains(self, address: u64, len: u64) -> bool {
        let mut index = 0;
        while index < self.count as usize {
            if self.windows[index].contains(address, len) {
                return true;
            }
            index += 1;
        }
        false
    }
}

impl DmaWindow {
    pub const fn new(base: u64, len: u64) -> Option<Self> {
        if len == 0 || base.checked_add(len).is_none() {
            None
        } else {
            Some(Self { base, len })
        }
    }

    pub const fn end(self) -> u64 {
        self.base + self.len
    }

    pub const fn contains(self, address: u64, len: u64) -> bool {
        match address.checked_add(len) {
            Some(end) => address >= self.base && end <= self.end(),
            None => false,
        }
    }
}

/// A GENET v5 descriptor as owned by the pure model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub address: u64,
    pub length: u32,
    pub status: u32,
}

/// The ownership and packet-boundary bits in a GENET descriptor status word.
///
/// The length occupies bits 16..=27; the lower control bits are deliberately
/// represented separately so callers cannot accidentally expose a raw status
/// word as a packet length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescriptorStatus {
    pub length: u16,
    pub ownership: Ownership,
    pub start: bool,
    pub end: bool,
    pub wrap: bool,
}

impl DescriptorStatus {
    pub const fn decode(word: u32) -> Result<Self, DescriptorError> {
        let length = ((word >> DMA_LENGTH_SHIFT) & DMA_LENGTH_MASK) as u16;
        if length == 0 {
            return Err(DescriptorError::Empty);
        }
        if length as u32 > MAX_FRAME_BYTES {
            return Err(DescriptorError::TooLarge);
        }
        Ok(Self {
            length,
            ownership: if word & DMA_OWN != 0 {
                Ownership::Device
            } else {
                Ownership::Driver
            },
            start: word & DMA_SOP != 0,
            end: word & DMA_EOP != 0,
            wrap: word & DMA_WRAP != 0,
        })
    }

    pub const fn encode(self) -> Result<u32, DescriptorError> {
        if self.length == 0 {
            return Err(DescriptorError::Empty);
        }
        if self.length as u32 > MAX_FRAME_BYTES || self.length as u32 > DMA_LENGTH_MASK {
            return Err(DescriptorError::TooLarge);
        }
        let mut word = (self.length as u32) << DMA_LENGTH_SHIFT;
        if matches!(self.ownership, Ownership::Device) {
            word |= DMA_OWN;
        }
        if self.start {
            word |= DMA_SOP;
        }
        if self.end {
            word |= DMA_EOP;
        }
        if self.wrap {
            word |= DMA_WRAP;
        }
        Ok(word)
    }
}

/// A descriptor ring's immutable address contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingLayout {
    pub base: u64,
    pub count: u16,
}

impl RingLayout {
    /// `base` is an offset in GENET's internal descriptor RAM, not a DMA PA.
    pub const fn new(base: u64, count: u16) -> Option<Self> {
        if (base != registers::RDMA as u64 && base != registers::TDMA as u64)
            || count == 0
            || count > TOTAL_DESCRIPTORS
            || !base.is_multiple_of(4)
        {
            return None;
        }
        let bytes = count as u64 * DESCRIPTOR_BYTES;
        if base.checked_add(bytes).is_none() || base + bytes > REGISTER_BYTES {
            return None;
        }
        Some(Self { base, count })
    }

    pub const fn descriptor_address(self, index: u16) -> Option<u64> {
        if index >= self.count {
            None
        } else {
            self.base.checked_add(index as u64 * DESCRIPTOR_BYTES)
        }
    }
}

/// Why a descriptor was refused before any device access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorError {
    AddressOutsideDma,
    AddressOverflow,
    Empty,
    TooLarge,
    WrongOwnership,
}

impl Descriptor {
    pub fn validate(self, dma: DmaWindow) -> Result<(), DescriptorError> {
        self.validate_address(|address, len| dma.contains(address, len))
    }

    pub fn validate_windows(self, dma: DmaWindows) -> Result<(), DescriptorError> {
        self.validate_address(|address, len| dma.contains(address, len))
    }

    fn validate_address(
        self,
        contains: impl FnOnce(u64, u64) -> bool,
    ) -> Result<(), DescriptorError> {
        if self.length == 0 {
            return Err(DescriptorError::Empty);
        }
        if self.length > MAX_FRAME_BYTES {
            return Err(DescriptorError::TooLarge);
        }
        if self.address.checked_add(self.length as u64).is_none() {
            return Err(DescriptorError::AddressOverflow);
        }
        if !contains(self.address, self.length as u64) {
            return Err(DescriptorError::AddressOutsideDma);
        }
        Ok(())
    }
}

/// Directional ownership of a descriptor slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    Driver,
    Device,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingError {
    Full,
    NoCompletion,
    InvalidStatus(DescriptorError),
    InvalidDescriptor(DescriptorError),
}

/// Bounded producer/consumer ownership for one GENET ring.
///
/// The hardware exposes producer and consumer indices, but it does not make
/// an out-of-order completion safe. This model keeps that ordering explicit:
/// the driver posts only at the producer cursor and reclaims only at the
/// consumer cursor. The fixed backing arrays are the model's bound, not a
/// request to allocate an unbounded queue.
pub struct RingState {
    layout: RingLayout,
    dma: DmaWindows,
    producer: u16,
    consumer: u16,
    ownership: [Ownership; TOTAL_DESCRIPTORS as usize],
    descriptors: [Option<Descriptor>; TOTAL_DESCRIPTORS as usize],
}

impl RingState {
    pub fn new(layout: RingLayout, dma: DmaWindows) -> Self {
        Self {
            layout,
            dma,
            producer: 0,
            consumer: 0,
            ownership: [Ownership::Driver; TOTAL_DESCRIPTORS as usize],
            descriptors: [None; TOTAL_DESCRIPTORS as usize],
        }
    }

    pub const fn producer(&self) -> u16 {
        self.producer
    }

    pub const fn consumer(&self) -> u16 {
        self.consumer
    }

    pub fn post(&mut self, descriptor: Descriptor) -> Result<u16, RingError> {
        descriptor
            .validate_windows(self.dma)
            .map_err(RingError::InvalidDescriptor)?;
        let index = self.producer;
        if self.ownership[index as usize] != Ownership::Driver {
            return Err(RingError::Full);
        }
        self.descriptors[index as usize] = Some(descriptor);
        self.ownership[index as usize] = Ownership::Device;
        self.producer = self.next(index);
        Ok(index)
    }

    pub fn complete(&mut self, status: u32) -> Result<(u16, Descriptor), RingError> {
        let status = DescriptorStatus::decode(status).map_err(RingError::InvalidStatus)?;
        if status.ownership != Ownership::Driver {
            return Err(RingError::InvalidStatus(DescriptorError::WrongOwnership));
        }
        let index = self.consumer;
        if self.ownership[index as usize] != Ownership::Device {
            return Err(RingError::NoCompletion);
        }
        let mut descriptor = self.descriptors[index as usize].ok_or(RingError::NoCompletion)?;
        descriptor.length = u32::from(status.length);
        descriptor.status = status.encode().map_err(RingError::InvalidStatus)?;
        descriptor
            .validate_windows(self.dma)
            .map_err(RingError::InvalidDescriptor)?;
        self.ownership[index as usize] = Ownership::Driver;
        self.consumer = self.next(index);
        Ok((index, descriptor))
    }

    const fn next(&self, index: u16) -> u16 {
        if index + 1 == self.layout.count {
            0
        } else {
            index + 1
        }
    }
}

/// A bounded ring cursor. The ring is fixed-size and never grows from device
/// input, so an out-of-range cursor is a refusal rather than a modulo guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingCursor {
    pub index: u16,
    pub ownership: Ownership,
}

impl RingCursor {
    pub const fn new(index: u16, ownership: Ownership) -> Option<Self> {
        if index < TOTAL_DESCRIPTORS {
            Some(Self { index, ownership })
        } else {
            None
        }
    }

    pub const fn advance(self) -> Self {
        Self {
            index: if self.index + 1 == TOTAL_DESCRIPTORS {
                0
            } else {
                self.index + 1
            },
            ownership: self.ownership,
        }
    }
}

/// The work classes raised by one GENET interrupt block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterruptWork {
    pub link: bool,
    pub mdio: bool,
    pub rx: bool,
    pub tx: bool,
}

impl InterruptWork {
    /// Classify and mask unknown bits. Unknown status is never interpreted as
    /// packet work; the caller may log/refuse it while acknowledging the raw
    /// status in the hardware-specific layer.
    pub const fn classify(status0: u32, status1: u32) -> Self {
        Self {
            link: status0 & interrupt::LINK_EVENT != 0,
            mdio: status0 & interrupt::MDIO_EVENT != 0,
            rx: status0 & interrupt::RX_DONE != 0
                || status1 & (0xffff << interrupt::QUEUE_RX_SHIFT) != 0,
            tx: status0 & interrupt::TX_DONE != 0 || status1 & interrupt::QUEUE_TX_MASK != 0,
        }
    }
}

/// Reset generations invalidate every descriptor token from the prior run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResetState {
    generation: u32,
    ready: bool,
}

impl ResetState {
    pub const fn new() -> Self {
        Self {
            generation: 1,
            ready: false,
        }
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }

    pub const fn ready(self) -> bool {
        self.ready
    }

    pub const fn activate(&mut self) {
        self.ready = true;
    }

    pub const fn reset(&mut self) {
        self.ready = false;
        self.generation = if self.generation == u32::MAX {
            1
        } else {
            self.generation + 1
        };
    }

    pub const fn accepts(self, generation: u32) -> bool {
        self.ready && self.generation == generation
    }
}

impl Default for ResetState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DMA: DmaWindow = DmaWindow {
        base: 0x1000,
        len: 0x4000,
    };
    const DMA_WINDOWS: DmaWindows = DmaWindows {
        windows: [DMA, DMA, DMA, DMA],
        count: 1,
    };

    #[test]
    fn dma_window_rejects_zero_and_wrapping_ranges() {
        assert_eq!(DmaWindow::new(0x1000, 0), None);
        assert_eq!(DmaWindow::new(u64::MAX, 2), None);
        assert!(DMA.contains(0x1000, 1));
        assert!(!DMA.contains(0x4fff, 2));
    }

    #[test]
    fn dma_windows_preserve_multiple_device_apertures() {
        let windows = DmaWindows::new(
            [
                DmaWindow::new(0x1000, 0x100).unwrap(),
                DmaWindow::new(0x4000, 0x100).unwrap(),
                DmaWindow::new(0x8000, 0x100).unwrap(),
                DmaWindow::new(0xc000, 0x100).unwrap(),
            ],
            2,
        )
        .unwrap();
        assert!(windows.contains(0x1080, 4));
        assert!(windows.contains(0x4080, 4));
        assert!(!windows.contains(0x8080, 4));
    }

    #[test]
    fn descriptor_validation_is_bounded_before_dma_access() {
        let good = Descriptor {
            address: 0x1800,
            length: 1500,
            status: 0,
        };
        assert_eq!(good.validate(DMA), Ok(()));
        assert_eq!(
            Descriptor { length: 0, ..good }.validate(DMA),
            Err(DescriptorError::Empty)
        );
        assert_eq!(
            Descriptor {
                length: MAX_FRAME_BYTES + 1,
                ..good
            }
            .validate(DMA),
            Err(DescriptorError::TooLarge)
        );
        assert_eq!(
            Descriptor {
                address: 0x4fff,
                length: 2,
                ..good
            }
            .validate(DMA),
            Err(DescriptorError::AddressOutsideDma)
        );
        assert_eq!(
            Descriptor {
                address: u64::MAX,
                length: 1,
                ..good
            }
            .validate(DMA),
            Err(DescriptorError::AddressOverflow)
        );
    }

    #[test]
    fn descriptor_status_round_trips_ownership_and_boundaries() {
        let status = DescriptorStatus {
            length: 1500,
            ownership: Ownership::Device,
            start: true,
            end: true,
            wrap: false,
        };
        assert_eq!(
            DescriptorStatus::decode(status.encode().unwrap()),
            Ok(status)
        );
        assert_eq!(
            DescriptorStatus::decode(DMA_SOP),
            Err(DescriptorError::Empty)
        );
        assert_eq!(
            DescriptorStatus {
                length: 0,
                ..status
            }
            .encode(),
            Err(DescriptorError::Empty)
        );
        assert_eq!(
            DescriptorStatus {
                length: MAX_FRAME_BYTES as u16 + 1,
                ..status
            }
            .encode(),
            Err(DescriptorError::TooLarge)
        );
    }

    #[test]
    fn ring_layout_checks_dma_span_and_index() {
        let ring = RingLayout::new(registers::RDMA as u64, 4).unwrap();
        assert_eq!(ring.descriptor_address(0), Some(0x2000));
        assert_eq!(ring.descriptor_address(3), Some(0x2024));
        assert_eq!(ring.descriptor_address(4), None);
        assert_eq!(RingLayout::new(REGISTER_BYTES, 1), None);
        assert_eq!(RingLayout::new(registers::RDMA as u64 + 1, 1), None);
    }

    #[test]
    fn ring_state_requires_ordered_post_and_completion() {
        let layout = RingLayout::new(registers::RDMA as u64, 2).unwrap();
        let mut ring = RingState::new(layout, DMA_WINDOWS);
        let descriptor = Descriptor {
            address: 0x1800,
            length: 1500,
            status: 0,
        };
        assert_eq!(
            ring.complete(0),
            Err(RingError::InvalidStatus(DescriptorError::Empty))
        );
        assert_eq!(ring.post(descriptor), Ok(0));
        assert_eq!(ring.consumer(), 0);
        let status = DescriptorStatus {
            length: 1500,
            ownership: Ownership::Driver,
            start: true,
            end: true,
            wrap: false,
        }
        .encode()
        .unwrap();
        let device_owned_status = DescriptorStatus {
            ownership: Ownership::Device,
            ..DescriptorStatus::decode(status).unwrap()
        }
        .encode()
        .unwrap();
        assert_eq!(
            ring.complete(device_owned_status),
            Err(RingError::InvalidStatus(DescriptorError::WrongOwnership))
        );
        let (index, completed) = ring.complete(status).unwrap();
        assert_eq!(index, 0);
        assert_eq!(completed.length, 1500);
        assert_eq!(ring.consumer(), 1);
        assert_eq!(ring.complete(status), Err(RingError::NoCompletion));
    }

    #[test]
    fn ring_state_refuses_full_ring_and_wraps_after_reclaim() {
        let layout = RingLayout::new(registers::TDMA as u64, 2).unwrap();
        let mut ring = RingState::new(layout, DMA_WINDOWS);
        let descriptor = Descriptor {
            address: 0x1800,
            length: 64,
            status: 0,
        };
        assert_eq!(ring.post(descriptor), Ok(0));
        assert_eq!(ring.post(descriptor), Ok(1));
        assert_eq!(ring.post(descriptor), Err(RingError::Full));
        let status = DescriptorStatus {
            length: 64,
            ownership: Ownership::Driver,
            start: true,
            end: true,
            wrap: false,
        }
        .encode()
        .unwrap();
        assert_eq!(ring.complete(status).unwrap().0, 0);
        assert_eq!(ring.post(descriptor), Ok(0));
    }

    #[test]
    fn ring_cursor_wraps_and_refuses_out_of_range_input() {
        assert_eq!(RingCursor::new(TOTAL_DESCRIPTORS, Ownership::Driver), None);
        let cursor = RingCursor::new(TOTAL_DESCRIPTORS - 1, Ownership::Device).unwrap();
        assert_eq!(
            cursor.advance(),
            RingCursor {
                index: 0,
                ownership: Ownership::Device
            }
        );
    }

    #[test]
    fn interrupt_classes_are_kept_directional() {
        let work = InterruptWork::classify(
            interrupt::LINK_EVENT | interrupt::MDIO_EVENT,
            (1 << interrupt::QUEUE_RX_SHIFT) | 1,
        );
        assert_eq!(
            work,
            InterruptWork {
                link: true,
                mdio: true,
                rx: true,
                tx: true
            }
        );
        assert!(InterruptWork::classify(interrupt::RX_DONE, 0).rx);
        assert!(InterruptWork::classify(interrupt::TX_DONE, 0).tx);
    }

    #[test]
    fn reset_invalidates_old_generation_until_activation() {
        let mut state = ResetState::new();
        let first = state.generation();
        assert!(!state.accepts(first));
        state.activate();
        assert!(state.accepts(first));
        state.reset();
        assert!(!state.accepts(first));
        assert!(!state.accepts(state.generation()));
        state.activate();
        assert!(state.accepts(state.generation()));
    }

    #[test]
    fn reset_generation_wrap_skips_zero() {
        let mut state = ResetState {
            generation: u32::MAX,
            ready: true,
        };
        state.reset();
        assert_eq!(state.generation(), 1);
    }
}
