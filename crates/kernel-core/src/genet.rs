//! Pure BCM2711 GENET v5 contracts (ADR-0106).
//!
//! This module deliberately stops at arithmetic and ownership. It does not
//! select a board address, touch MMIO, or expose a descriptor to EL0. The
//! eventual Pi 4 binding must supply a verified device-tree translation and
//! use these checks before programming the controller.

use core::fmt::{self, Display, Formatter};

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
pub const GENET_V5_MAJOR: u8 = 5;

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
    pub const INTRL2_CPU_CLEAR: u32 = 0x08;
    pub const INTRL2_CPU_MASK_SET: u32 = 0x10;
    pub const RBUF_CTRL: u32 = RBUF;
    pub const UMAC_CMD: u32 = UMAC + 0x08;
    pub const UMAC_TX_FLUSH: u32 = UMAC + 0x334;
    pub const UMAC_MDIO_CMD: u32 = MDIO;
}

/// Clause-22 MDIO command word at [`registers::UMAC_MDIO_CMD`].
pub mod mdio {
    pub const START_BUSY: u32 = 1 << 29;
    pub const READ_FAIL: u32 = 1 << 28;
    pub const READ: u32 = 2 << 26;
    pub const WRITE: u32 = 1 << 26;
    pub const PHY_SHIFT: u32 = 21;
    pub const PHY_MASK: u32 = 0x1f;
    pub const REG_SHIFT: u32 = 16;
    pub const REG_MASK: u32 = 0x1f;
    pub const DATA_MASK: u32 = 0xffff;
    pub const PHYIDR1: u8 = 2;
    pub const PHYIDR2: u8 = 3;
}

/// IEEE 802.3 clause-22 PHY registers used by the unpublished bring-up.
pub mod phy {
    pub const BMCR: u8 = 0;
    pub const BMSR: u8 = 1;
    pub const BMCR_RESET: u16 = 1 << 15;
    pub const BMCR_ANENABLE: u16 = 1 << 12;
    pub const BMCR_ANRESTART: u16 = 1 << 9;
    pub const BMSR_LINK: u16 = 1 << 2;
    pub const BMSR_ANEG_DONE: u16 = 1 << 5;
}

/// GENET v5 register layout shared by the RDMA and TDMA blocks.
///
/// Descriptor RAM occupies the first [`DESCRIPTOR_RAM_BYTES`] of each
/// block. Per-ring registers start after that RAM; the common control
/// block follows all 17 rings. Offsets are relative to [`registers::RDMA`]
/// or [`registers::TDMA`].
pub mod dma_registers {
    use super::{DESCRIPTOR_BYTES, TOTAL_DESCRIPTORS};

    pub const RING_BYTES: u32 = 0x40;
    pub const RING_COUNT: u16 = 17;
    /// v4+ descriptor is three 32-bit words; start/end pointers are in words.
    pub const WORDS_PER_DESCRIPTOR: u32 = 3;
    pub const DESCRIPTOR_RAM_BYTES: u32 = TOTAL_DESCRIPTORS as u32 * DESCRIPTOR_BYTES as u32;
    pub const RING_BASE: u32 = DESCRIPTOR_RAM_BYTES;
    pub const COMMON_BASE: u32 = RING_BASE + RING_BYTES * RING_COUNT as u32;
    pub const CTRL: u32 = COMMON_BASE + 0x04;
    pub const STATUS: u32 = COMMON_BASE + 0x08;
    pub const SCB_BURST_SIZE: u32 = COMMON_BASE + 0x0c;
    pub const ARB_CTRL: u32 = COMMON_BASE + 0x2c;
    pub const RING_CFG: u32 = COMMON_BASE;
    pub const RING0: u32 = RING_BASE;
    pub const DMA_ENABLE: u32 = 1 << 0;
    pub const RING_BUF_EN_SHIFT: u32 = 1;

    /// Per-ring offsets for the v4+ 40-bit pointer layout.
    pub const READ_PTR: u32 = 0x00;
    pub const READ_PTR_HI: u32 = 0x04;
    pub const CONS_INDEX: u32 = 0x08;
    pub const PROD_INDEX: u32 = 0x0c;
    pub const RING_BUF_SIZE: u32 = 0x10;
    pub const START_ADDR: u32 = 0x14;
    pub const START_ADDR_HI: u32 = 0x18;
    pub const END_ADDR: u32 = 0x1c;
    pub const END_ADDR_HI: u32 = 0x20;
    pub const MBUF_DONE_THRESH: u32 = 0x24;
    pub const FLOW_PERIOD: u32 = 0x28;
    pub const WRITE_PTR: u32 = 0x2c;
    pub const WRITE_PTR_HI: u32 = 0x30;
    pub const RING_SIZE_SHIFT: u32 = 16;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision {
    pub major: u8,
    pub minor: u8,
    pub patch: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevisionError {
    Unsupported(u8),
}

impl Revision {
    pub const fn decode(raw: u32) -> Result<Self, RevisionError> {
        let encoded = ((raw >> 24) & 0x0f) as u8;
        // SYS_REV_CTRL [27:24] is not the logical GENET major. Linux
        // `bcmgenet_set_hw_params` remaps encoded 6/7 → logical v5,
        // encoded 5 → v4, encoded 0 → v1. This product models only the
        // v5 descriptor family, so 6/7 are accepted and the chip's
        // encoded major is what the report prints. This Pi 4B reported 6
        // (`.serial-log/20260814-140651.log`, `src=92c889f4`).
        if encoded != 6 && encoded != 7 {
            return Err(RevisionError::Unsupported(encoded));
        }
        Ok(Self {
            major: encoded,
            minor: ((raw >> 16) & 0x0f) as u8,
            patch: (raw & 0xffff) as u16,
        })
    }
}

/// Outcome of the compiled-window GENET bring-up. Not a NIC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmioProbe {
    NoBinding,
    OutsideWindow,
    NotPresent,
    Unsupported(u8),
    Timeout,
    InvalidBinding,
    Revision(Revision),
}

/// Compare an FDT-translated register window to the compiled BSP claim.
///
/// ADR-0072: the kernel maps the compiled window, then verifies the binding.
/// A discovered PA is never used as a map base.
pub const fn matches_compiled_window(
    mmio_base: u64,
    mmio_len: u64,
    compiled_base: u64,
    compiled_len: u64,
) -> bool {
    mmio_base == compiled_base && mmio_len == compiled_len
}

/// Decide whether the compiled window may be probed.
///
/// `Ok(())` means the binding matches and the caller must run `Genet::probe`.
/// `Err` is the boot line: do not invent a register word.
pub const fn mmio_probe_intent(
    binding: Option<(u64, u64)>,
    compiled_base: u64,
    compiled_len: u64,
) -> Result<(), MmioProbe> {
    match binding {
        None => Err(MmioProbe::NoBinding),
        Some((base, len)) if !matches_compiled_window(base, len, compiled_base, compiled_len) => {
            Err(MmioProbe::OutsideWindow)
        }
        Some(_) => Ok(()),
    }
}

impl Display for MmioProbe {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            MmioProbe::NoBinding => f.write_str("genet: probe unavailable (no binding)"),
            MmioProbe::OutsideWindow => {
                f.write_str("genet: probe skipped (binding outside compiled window)")
            }
            MmioProbe::NotPresent => f.write_str("genet: probe unavailable (NotPresent)"),
            MmioProbe::Unsupported(major) => {
                write!(f, "genet: probe unavailable (Unsupported({major}))")
            }
            MmioProbe::Timeout => f.write_str("genet: probe unavailable (Timeout)"),
            MmioProbe::InvalidBinding => f.write_str("genet: probe unavailable (InvalidBinding)"),
            MmioProbe::Revision(revision) => write!(
                f,
                "genet: rev={}.{} patch={:#x} (mmio, not a nic)",
                revision.major, revision.minor, revision.patch
            ),
        }
    }
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
///
/// `base` is the device-visible (child) address written into descriptors.
/// `cpu_base` is the matching CPU physical address (parent). Identity
/// maps set them equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaWindow {
    pub base: u64,
    pub cpu_base: u64,
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

    /// Translate a CPU physical address into the device DMA address.
    pub const fn map_cpu(self, cpu: u64, len: u64) -> Result<u64, DmaMapError> {
        let mut index = 0;
        while index < self.count as usize {
            if let Some(dma) = self.windows[index].map_cpu(cpu, len) {
                return Ok(dma);
            }
            index += 1;
        }
        Err(DmaMapError::OutsideWindow)
    }
}

/// Why a CPU→DMA translation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmaMapError {
    OutsideWindow,
}

impl DmaWindow {
    pub const fn new(base: u64, len: u64) -> Option<Self> {
        Self::mapped(base, base, len)
    }

    pub const fn mapped(dma_base: u64, cpu_base: u64, len: u64) -> Option<Self> {
        if len == 0 || dma_base.checked_add(len).is_none() || cpu_base.checked_add(len).is_none() {
            None
        } else {
            Some(Self {
                base: dma_base,
                cpu_base,
                len,
            })
        }
    }

    pub const fn map_cpu(self, cpu: u64, len: u64) -> Option<u64> {
        match cpu.checked_add(len) {
            Some(end) if cpu >= self.cpu_base && end <= self.cpu_base + self.len => {
                Some(self.base + (cpu - self.cpu_base))
            }
            _ => None,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescriptorWords {
    pub length_status: u32,
    pub address_low: u32,
    pub address_high: u32,
}

impl Descriptor {
    pub fn words(
        self,
        ownership: Ownership,
        start: bool,
        end: bool,
        wrap: bool,
    ) -> Result<DescriptorWords, DescriptorError> {
        self.validate_address(|_, _| true)?;
        let status = DescriptorStatus {
            length: self.length as u16,
            ownership,
            start,
            end,
            wrap,
        }
        .encode()?;
        Ok(DescriptorWords {
            length_status: status,
            address_low: self.address as u32,
            address_high: (self.address >> 32) as u32,
        })
    }
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

/// Why a queue-0 ring program was refused before any MMIO write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingProgramError {
    InvalidBlock,
    UnsupportedQueue,
    Empty,
    TooMany,
    FirstOutOfRange,
    SpanOverflow,
    BufferEmpty,
    BufferTooLarge,
}

/// Immutable v5 program for one queue-0 ring in descriptor RAM.
///
/// `start`/`end` are word offsets into the controller's internal descriptor
/// RAM (`first * 3` .. `(first + count) * 3 - 1`). Packet-buffer DMA
/// addresses are validated separately and never become a ring base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingProgram {
    pub block: u32,
    pub queue: u8,
    pub count: u16,
    pub first: u16,
    pub buffer_bytes: u16,
}

/// Register values a programmed ring writes, in v4+ layout order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingProgramWords {
    pub prod: u32,
    pub cons: u32,
    pub ring_buf_size: u32,
    pub start: u32,
    pub start_hi: u32,
    pub end: u32,
    pub end_hi: u32,
    pub mbuf_done: u32,
    pub flow: u32,
    pub read_ptr: u32,
    pub write_ptr: u32,
}

impl RingProgram {
    pub const fn new(
        block: u32,
        queue: u8,
        first: u16,
        count: u16,
        buffer_bytes: u16,
    ) -> Result<Self, RingProgramError> {
        if block != registers::RDMA && block != registers::TDMA {
            return Err(RingProgramError::InvalidBlock);
        }
        if queue != 0 {
            return Err(RingProgramError::UnsupportedQueue);
        }
        if count == 0 {
            return Err(RingProgramError::Empty);
        }
        if count > TOTAL_DESCRIPTORS {
            return Err(RingProgramError::TooMany);
        }
        if first >= TOTAL_DESCRIPTORS {
            return Err(RingProgramError::FirstOutOfRange);
        }
        if first as u32 + count as u32 > TOTAL_DESCRIPTORS as u32 {
            return Err(RingProgramError::SpanOverflow);
        }
        if buffer_bytes == 0 {
            return Err(RingProgramError::BufferEmpty);
        }
        if buffer_bytes as u32 > MAX_FRAME_BYTES {
            return Err(RingProgramError::BufferTooLarge);
        }
        Ok(Self {
            block,
            queue,
            count,
            first,
            buffer_bytes,
        })
    }

    pub const fn ring_register_base(self) -> u32 {
        self.block + dma_registers::RING_BASE + self.queue as u32 * dma_registers::RING_BYTES
    }

    pub const fn start_words(self) -> u32 {
        self.first as u32 * dma_registers::WORDS_PER_DESCRIPTOR
    }

    pub const fn end_words(self) -> u32 {
        (self.first as u32 + self.count as u32) * dma_registers::WORDS_PER_DESCRIPTOR - 1
    }

    pub const fn ring_buf_size(self) -> u32 {
        (self.count as u32) << dma_registers::RING_SIZE_SHIFT | self.buffer_bytes as u32
    }

    pub const fn words(self) -> RingProgramWords {
        let start = self.start_words();
        RingProgramWords {
            prod: 0,
            cons: 0,
            ring_buf_size: self.ring_buf_size(),
            start,
            start_hi: 0,
            end: self.end_words(),
            end_hi: 0,
            mbuf_done: 1,
            flow: 0,
            read_ptr: start,
            write_ptr: start,
        }
    }
}

/// Why a clause-22 MDIO word was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MdioError {
    PhyOutOfRange,
    RegisterOutOfRange,
    Busy,
    ReadFail,
    AbsentPhyId,
    StuckHighPhyId,
}

/// One clause-22 MDIO transaction. `write = None` is a read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MdioTxn {
    pub phy: u8,
    pub reg: u8,
    pub write: Option<u16>,
}

impl MdioTxn {
    pub const fn new(phy: u8, reg: u8, write: Option<u16>) -> Result<Self, MdioError> {
        if phy > mdio::PHY_MASK as u8 {
            return Err(MdioError::PhyOutOfRange);
        }
        if reg > mdio::REG_MASK as u8 {
            return Err(MdioError::RegisterOutOfRange);
        }
        Ok(Self { phy, reg, write })
    }

    pub const fn encode(self) -> Result<u32, MdioError> {
        if self.phy > mdio::PHY_MASK as u8 {
            return Err(MdioError::PhyOutOfRange);
        }
        if self.reg > mdio::REG_MASK as u8 {
            return Err(MdioError::RegisterOutOfRange);
        }
        let mut word = mdio::START_BUSY
            | (self.phy as u32) << mdio::PHY_SHIFT
            | (self.reg as u32) << mdio::REG_SHIFT;
        match self.write {
            Some(data) => word |= mdio::WRITE | data as u32,
            None => word |= mdio::READ,
        }
        Ok(word)
    }

    pub const fn decode_read(word: u32) -> Result<u16, MdioError> {
        if word & mdio::START_BUSY != 0 {
            return Err(MdioError::Busy);
        }
        if word & mdio::READ_FAIL != 0 {
            return Err(MdioError::ReadFail);
        }
        Ok((word & mdio::DATA_MASK) as u16)
    }
}

/// Combine PHYIDR1/PHYIDR2. All-zero is absent; all-ones is a stuck bus.
pub const fn classify_phy_id(hi: u16, lo: u16) -> Result<u32, MdioError> {
    if hi == 0 && lo == 0 {
        Err(MdioError::AbsentPhyId)
    } else if hi == 0xffff && lo == 0xffff {
        Err(MdioError::StuckHighPhyId)
    } else {
        Ok(((hi as u32) << 16) | lo as u32)
    }
}

/// Why a PHY identify / link classify was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhyError {
    ModeNotRgmiiRxid,
    Id(MdioError),
    ResetPending,
    LinkDown,
}

/// Clause-22 link snapshot. `rgmii-rxid` is a binding fact, not a delay table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhyLink {
    pub id: u32,
    pub rgmii_rxid: bool,
    pub state: LinkState,
}

impl PhyLink {
    pub const fn identify(hi: u16, lo: u16, rgmii_rxid: bool) -> Result<Self, PhyError> {
        if !rgmii_rxid {
            return Err(PhyError::ModeNotRgmiiRxid);
        }
        match classify_phy_id(hi, lo) {
            Ok(id) => Ok(Self {
                id,
                rgmii_rxid: true,
                state: LinkState::Down,
            }),
            Err(error) => Err(PhyError::Id(error)),
        }
    }

    pub const fn reset_command() -> u16 {
        phy::BMCR_RESET
    }

    pub const fn reset_cleared(bmcr: u16) -> Result<(), PhyError> {
        if bmcr & phy::BMCR_RESET == 0 {
            Ok(())
        } else {
            Err(PhyError::ResetPending)
        }
    }

    pub const fn classify_bmsr(bmsr: u16) -> LinkState {
        if bmsr & phy::BMSR_LINK != 0 {
            LinkState::Up
        } else {
            LinkState::Down
        }
    }

    pub const fn with_bmsr(self, bmsr: u16) -> Self {
        Self {
            state: Self::classify_bmsr(bmsr),
            ..self
        }
    }

    pub const fn require_up(self) -> Result<Self, PhyError> {
        match self.state {
            LinkState::Up => Ok(self),
            LinkState::Down => Err(PhyError::LinkDown),
        }
    }
}

/// Boot report for a clause-22 PHY identify. Not a NIC and not a link claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhyIdentify {
    Identity(PhyLink),
    Unavailable(PhyError),
    Timeout,
}

impl PhyIdentify {
    pub const fn from_identify(hi: u16, lo: u16, rgmii_rxid: bool) -> Self {
        match PhyLink::identify(hi, lo, rgmii_rxid) {
            Ok(link) => Self::Identity(link),
            Err(error) => Self::Unavailable(error),
        }
    }
}

impl Display for PhyIdentify {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            PhyIdentify::Identity(link) => {
                write!(f, "genet: phy={:#010x} (id, not a nic)", link.id)
            }
            PhyIdentify::Unavailable(PhyError::ModeNotRgmiiRxid) => {
                f.write_str("genet: phy unavailable (mode)")
            }
            PhyIdentify::Unavailable(PhyError::Id(MdioError::AbsentPhyId)) => {
                f.write_str("genet: phy unavailable (absent id)")
            }
            PhyIdentify::Unavailable(PhyError::Id(MdioError::StuckHighPhyId)) => {
                f.write_str("genet: phy unavailable (stuck-high id)")
            }
            PhyIdentify::Unavailable(PhyError::Id(MdioError::Busy)) => {
                f.write_str("genet: phy unavailable (busy)")
            }
            PhyIdentify::Unavailable(PhyError::Id(MdioError::ReadFail)) => {
                f.write_str("genet: phy unavailable (read fail)")
            }
            PhyIdentify::Unavailable(PhyError::Id(_)) => f.write_str("genet: phy unavailable (id)"),
            PhyIdentify::Unavailable(PhyError::ResetPending) => {
                f.write_str("genet: phy unavailable (reset pending)")
            }
            PhyIdentify::Unavailable(PhyError::LinkDown) => {
                f.write_str("genet: phy unavailable (link down)")
            }
            PhyIdentify::Timeout => f.write_str("genet: phy unavailable (timeout)"),
        }
    }
}

/// Boot report for a BMSR link snapshot. Not a NIC and not a service bind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkReport {
    Classified(LinkState),
    Unavailable(MdioError),
    Timeout,
}

impl LinkReport {
    pub const fn from_bmsr(bmsr: u16) -> Self {
        Self::Classified(PhyLink::classify_bmsr(bmsr))
    }
}

impl Display for LinkReport {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            LinkReport::Classified(LinkState::Up) => {
                f.write_str("genet: link=up (bmsr, not a nic)")
            }
            LinkReport::Classified(LinkState::Down) => {
                f.write_str("genet: link=down (bmsr, not a nic)")
            }
            LinkReport::Unavailable(MdioError::Busy) => {
                f.write_str("genet: link unavailable (busy)")
            }
            LinkReport::Unavailable(MdioError::ReadFail) => {
                f.write_str("genet: link unavailable (read fail)")
            }
            LinkReport::Unavailable(_) => f.write_str("genet: link unavailable (mdio)"),
            LinkReport::Timeout => f.write_str("genet: link unavailable (timeout)"),
        }
    }
}

/// Boot report for a programmed queue-0 pair. Not a NIC: DMA stays disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Queue0Report {
    Programmed,
    Enabled,
    OutsideDma,
    NoFrames,
    Descriptor(DescriptorError),
    Ring(RingProgramError),
    Enable(QueueEnableError),
}

impl Display for Queue0Report {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Queue0Report::Programmed => f.write_str("genet: queue0 programmed (rings, not a nic)"),
            Queue0Report::Enabled => f.write_str("genet: queue0 enabled (dma, not a nic)"),
            Queue0Report::OutsideDma => f.write_str("genet: queue0 unavailable (outside dma)"),
            Queue0Report::NoFrames => f.write_str("genet: queue0 unavailable (no frames)"),
            Queue0Report::Descriptor(_) => f.write_str("genet: queue0 unavailable (descriptor)"),
            Queue0Report::Ring(_) => f.write_str("genet: queue0 unavailable (ring)"),
            Queue0Report::Enable(QueueEnableError::NotProgrammed) => {
                f.write_str("genet: queue0 unavailable (not programmed)")
            }
            Queue0Report::Enable(QueueEnableError::AlreadyEnabled) => {
                f.write_str("genet: queue0 unavailable (already enabled)")
            }
            Queue0Report::Enable(_) => f.write_str("genet: queue0 unavailable (phase)"),
        }
    }
}

/// Why a queue-0 DMA enable word was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueEnableError {
    UnsupportedQueue,
    NotProgrammed,
    AlreadyEnabled,
}

/// v5 common-block words that turn on queue 0 after the rings are programmed.
///
/// `RING_CFG` names the ring; `CTRL` then sets both `DMA_EN` and that ring's
/// buffer-enable bit. Hardware completions live at [`dma_registers::CONS_INDEX`];
/// software producer/consumer updates live at [`dma_registers::PROD_INDEX`]
/// for both engines — the merged v4+ map aliases RDMA_PROD onto CONS and
/// RDMA_CONS onto PROD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueEnable {
    pub queue: u8,
}

impl QueueEnable {
    pub const fn new(queue: u8) -> Result<Self, QueueEnableError> {
        if queue != 0 {
            return Err(QueueEnableError::UnsupportedQueue);
        }
        Ok(Self { queue })
    }

    pub const fn ring_cfg(self) -> u32 {
        1 << self.queue
    }

    pub const fn ctrl(self) -> u32 {
        dma_registers::DMA_ENABLE | (1 << (dma_registers::RING_BUF_EN_SHIFT + self.queue as u32))
    }

    pub const fn hardware_index(self) -> u32 {
        dma_registers::CONS_INDEX
    }

    pub const fn software_index(self) -> u32 {
        dma_registers::PROD_INDEX
    }
}

/// Driver-side phase so enable cannot run before program or twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DmaPhase {
    Idle,
    Programmed,
    Enabled,
}

impl DmaPhase {
    pub const fn program(self) -> Result<Self, QueueEnableError> {
        match self {
            Self::Idle | Self::Programmed => Ok(Self::Programmed),
            Self::Enabled => Err(QueueEnableError::AlreadyEnabled),
        }
    }

    pub const fn enable(self) -> Result<Self, QueueEnableError> {
        match self {
            Self::Programmed => Ok(Self::Enabled),
            Self::Idle => Err(QueueEnableError::NotProgrammed),
            Self::Enabled => Err(QueueEnableError::AlreadyEnabled),
        }
    }

    pub const fn reset(self) -> Self {
        Self::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DMA: DmaWindow = DmaWindow {
        base: 0x1000,
        cpu_base: 0x1000,
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
        assert_eq!(DMA.map_cpu(0x1100, 16), Some(0x1100));
        let aliased = DmaWindow::mapped(0x4_0000_0000, 0, 0x4000_0000).unwrap();
        assert_eq!(aliased.map_cpu(0x411d000, 64), Some(0x4_0411_d000));
        assert_eq!(
            DmaWindows::new([aliased, aliased, aliased, aliased], 1)
                .unwrap()
                .map_cpu(0x9000_0000, 64),
            Err(DmaMapError::OutsideWindow)
        );
    }

    #[test]
    fn v5_revision_and_dma_register_layout_are_explicit() {
        let revision = Revision::decode(6 << 24 | 3 << 16 | 0x2711).unwrap();
        assert_eq!(revision.major, 6);
        assert_eq!(revision.minor, 3);
        assert_eq!(revision.patch, 0x2711);
        assert_eq!(dma_registers::DESCRIPTOR_RAM_BYTES, 0xc00);
        assert_eq!(dma_registers::RING_BASE, 0xc00);
        assert_eq!(dma_registers::COMMON_BASE, 0x1040);
        assert_eq!(dma_registers::CTRL, 0x1044);
        assert_eq!(dma_registers::RING0, 0xc00);
        assert_eq!(
            Revision::decode(4 << 24),
            Err(RevisionError::Unsupported(4))
        );
        // Encoded 5 is GENET v4, not v5.
        assert_eq!(
            Revision::decode(5 << 24),
            Err(RevisionError::Unsupported(5))
        );
        let six = Revision::decode(6 << 24 | 1 << 16).unwrap();
        assert_eq!(six.major, 6);
        assert_eq!(six.minor, 1);
        assert!(Revision::decode(7 << 24).is_ok());
        assert_eq!(
            Revision::decode(8 << 24),
            Err(RevisionError::Unsupported(8))
        );
    }

    #[test]
    fn mmio_probe_is_classified_against_the_compiled_window() {
        assert!(matches_compiled_window(
            0xfd58_0000,
            REGISTER_BYTES,
            0xfd58_0000,
            REGISTER_BYTES
        ));
        assert!(!matches_compiled_window(
            0xfd58_0000,
            REGISTER_BYTES,
            0xfe00_0000,
            REGISTER_BYTES
        ));
        assert_eq!(
            mmio_probe_intent(None, 0xfd58_0000, REGISTER_BYTES),
            Err(MmioProbe::NoBinding)
        );
        assert_eq!(
            mmio_probe_intent(
                Some((0xfe00_0000, REGISTER_BYTES)),
                0xfd58_0000,
                REGISTER_BYTES
            ),
            Err(MmioProbe::OutsideWindow)
        );
        assert_eq!(
            mmio_probe_intent(
                Some((0xfd58_0000, REGISTER_BYTES)),
                0xfd58_0000,
                REGISTER_BYTES
            ),
            Ok(())
        );
        assert_eq!(
            MmioProbe::NoBinding.to_string(),
            "genet: probe unavailable (no binding)"
        );
        let raw = 6 << 24 | 3 << 16 | 0x2711;
        assert_eq!(
            MmioProbe::Revision(Revision::decode(raw).unwrap()).to_string(),
            "genet: rev=6.3 patch=0x2711 (mmio, not a nic)"
        );
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
        let words = Descriptor {
            address: 0x1_0000_2000,
            length: 1500,
            status: 0,
        }
        .words(Ownership::Device, true, true, false)
        .unwrap();
        assert_eq!(words.address_low, 0x0000_2000);
        assert_eq!(words.address_high, 1);
        assert_eq!(
            DescriptorStatus::decode(words.length_status)
                .unwrap()
                .ownership,
            Ownership::Device
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

    #[test]
    fn ring_program_encodes_v5_queue0_in_descriptor_word_units() {
        let program = RingProgram::new(registers::TDMA, 0, 0, 2, 128).unwrap();
        assert_eq!(program.ring_register_base(), registers::TDMA + 0xc00);
        assert_eq!(program.start_words(), 0);
        assert_eq!(program.end_words(), 5);
        assert_eq!(program.ring_buf_size(), 2 << 16 | 128);
        let words = program.words();
        assert_eq!(words.prod, 0);
        assert_eq!(words.cons, 0);
        assert_eq!(words.start, 0);
        assert_eq!(words.end, 5);
        assert_eq!(words.read_ptr, 0);
        assert_eq!(words.write_ptr, 0);
        assert_eq!(words.mbuf_done, 1);
        let shifted = RingProgram::new(registers::RDMA, 0, 4, 1, 64).unwrap();
        assert_eq!(shifted.start_words(), 12);
        assert_eq!(shifted.end_words(), 14);
        assert_eq!(
            shifted.ring_register_base(),
            registers::RDMA + dma_registers::RING0
        );
    }

    #[test]
    fn ring_program_refuses_bad_queue_span_and_buffer() {
        assert_eq!(
            RingProgram::new(registers::RDMA, 1, 0, 1, 64),
            Err(RingProgramError::UnsupportedQueue)
        );
        assert_eq!(
            RingProgram::new(registers::RDMA, 0, 0, 0, 64),
            Err(RingProgramError::Empty)
        );
        assert_eq!(
            RingProgram::new(registers::RDMA, 0, 0, TOTAL_DESCRIPTORS + 1, 64),
            Err(RingProgramError::TooMany)
        );
        assert_eq!(
            RingProgram::new(registers::RDMA, 0, TOTAL_DESCRIPTORS, 1, 64),
            Err(RingProgramError::FirstOutOfRange)
        );
        assert_eq!(
            RingProgram::new(registers::RDMA, 0, TOTAL_DESCRIPTORS - 1, 2, 64),
            Err(RingProgramError::SpanOverflow)
        );
        assert_eq!(
            RingProgram::new(registers::RDMA, 0, 0, 1, 0),
            Err(RingProgramError::BufferEmpty)
        );
        assert_eq!(
            RingProgram::new(registers::RDMA, 0, 0, 1, MAX_FRAME_BYTES as u16 + 1),
            Err(RingProgramError::BufferTooLarge)
        );
        assert_eq!(
            RingProgram::new(0, 0, 0, 1, 64),
            Err(RingProgramError::InvalidBlock)
        );
    }

    #[test]
    fn ring_program_does_not_reuse_a_packet_buffer_as_ring_base() {
        let program = RingProgram::new(registers::RDMA, 0, 0, 1, 128).unwrap();
        let packet = Descriptor {
            address: 0x1800,
            length: 128,
            status: 0,
        };
        assert_eq!(packet.validate(DMA), Ok(()));
        assert_ne!(u64::from(program.ring_register_base()), packet.address);
        assert_ne!(u64::from(program.start_words()), packet.address);
    }

    #[test]
    fn mdio_txn_encodes_clause22_and_classifies_phy_id() {
        let read = MdioTxn::new(1, mdio::PHYIDR1, None).unwrap();
        let word = read.encode().unwrap();
        assert_ne!(word & mdio::START_BUSY, 0);
        assert_ne!(word & mdio::READ, 0);
        assert_eq!(word & mdio::WRITE, 0);
        assert_eq!((word >> mdio::PHY_SHIFT) & mdio::PHY_MASK, 1);
        assert_eq!((word >> mdio::REG_SHIFT) & mdio::REG_MASK, 2);
        assert_eq!(MdioTxn::decode_read(0x600d), Ok(0x600d));
        assert_eq!(MdioTxn::decode_read(mdio::START_BUSY), Err(MdioError::Busy));
        assert_eq!(
            MdioTxn::decode_read(mdio::READ_FAIL | 1),
            Err(MdioError::ReadFail)
        );
        let write = MdioTxn::new(1, 0, Some(0x8000)).unwrap().encode().unwrap();
        assert_ne!(write & mdio::WRITE, 0);
        assert_eq!(write & mdio::DATA_MASK, 0x8000);
        assert_eq!(classify_phy_id(0x0362, 0x5e60), Ok(0x0362_5e60));
        assert_eq!(classify_phy_id(0, 0), Err(MdioError::AbsentPhyId));
        assert_eq!(
            classify_phy_id(0xffff, 0xffff),
            Err(MdioError::StuckHighPhyId)
        );
        assert_eq!(MdioTxn::new(32, 0, None), Err(MdioError::PhyOutOfRange));
        assert_eq!(
            MdioTxn::new(0, 32, None),
            Err(MdioError::RegisterOutOfRange)
        );
    }

    #[test]
    fn phy_link_identifies_and_classifies_bmsr() {
        let link = PhyLink::identify(0x0362, 0x5e60, true).unwrap();
        assert_eq!(link.id, 0x0362_5e60);
        assert!(link.rgmii_rxid);
        assert_eq!(link.state, LinkState::Down);
        assert_eq!(PhyLink::reset_command(), phy::BMCR_RESET);
        assert_eq!(PhyLink::reset_cleared(0), Ok(()));
        assert_eq!(
            PhyLink::reset_cleared(phy::BMCR_RESET),
            Err(PhyError::ResetPending)
        );
        assert_eq!(PhyLink::classify_bmsr(0), LinkState::Down);
        assert_eq!(PhyLink::classify_bmsr(phy::BMSR_LINK), LinkState::Up);
        assert_eq!(
            LinkReport::from_bmsr(0).to_string(),
            "genet: link=down (bmsr, not a nic)"
        );
        assert_eq!(
            LinkReport::from_bmsr(phy::BMSR_LINK).to_string(),
            "genet: link=up (bmsr, not a nic)"
        );
        assert_eq!(
            LinkReport::Timeout.to_string(),
            "genet: link unavailable (timeout)"
        );
        assert_eq!(
            link.with_bmsr(phy::BMSR_LINK).require_up().unwrap().state,
            LinkState::Up
        );
        assert_eq!(link.with_bmsr(0).require_up(), Err(PhyError::LinkDown));
    }

    #[test]
    fn phy_link_refuses_wrong_mode_and_absent_id() {
        assert_eq!(
            PhyLink::identify(0x0362, 0x5e60, false),
            Err(PhyError::ModeNotRgmiiRxid)
        );
        assert_eq!(
            PhyLink::identify(0, 0, true),
            Err(PhyError::Id(MdioError::AbsentPhyId))
        );
        assert_eq!(
            PhyLink::identify(0xffff, 0xffff, true),
            Err(PhyError::Id(MdioError::StuckHighPhyId))
        );
        assert_eq!(
            PhyIdentify::from_identify(0x0362, 0x5e60, true).to_string(),
            "genet: phy=0x03625e60 (id, not a nic)"
        );
        assert_eq!(
            PhyIdentify::from_identify(0, 0, true).to_string(),
            "genet: phy unavailable (absent id)"
        );
        assert_eq!(
            PhyIdentify::from_identify(0xffff, 0xffff, true).to_string(),
            "genet: phy unavailable (stuck-high id)"
        );
        assert_eq!(
            PhyIdentify::from_identify(0x0362, 0x5e60, false).to_string(),
            "genet: phy unavailable (mode)"
        );
        assert_eq!(
            PhyIdentify::Timeout.to_string(),
            "genet: phy unavailable (timeout)"
        );
    }

    #[test]
    fn queue_enable_is_queue0_and_keeps_index_polarity() {
        let enable = QueueEnable::new(0).unwrap();
        assert_eq!(enable.ring_cfg(), 1);
        assert_eq!(
            enable.ctrl(),
            dma_registers::DMA_ENABLE | (1 << dma_registers::RING_BUF_EN_SHIFT)
        );
        assert_eq!(enable.hardware_index(), dma_registers::CONS_INDEX);
        assert_eq!(enable.software_index(), dma_registers::PROD_INDEX);
        assert_eq!(QueueEnable::new(1), Err(QueueEnableError::UnsupportedQueue));
        assert_eq!(DmaPhase::Idle.program(), Ok(DmaPhase::Programmed));
        assert_eq!(
            DmaPhase::Idle.enable(),
            Err(QueueEnableError::NotProgrammed)
        );
        assert_eq!(DmaPhase::Programmed.enable(), Ok(DmaPhase::Enabled));
        assert_eq!(
            DmaPhase::Enabled.enable(),
            Err(QueueEnableError::AlreadyEnabled)
        );
        assert_eq!(DmaPhase::Enabled.reset(), DmaPhase::Idle);
    }
}
