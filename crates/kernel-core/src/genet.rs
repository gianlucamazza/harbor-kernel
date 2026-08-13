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

/// Why a descriptor was refused before any device access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorError {
    AddressOutsideDma,
    AddressOverflow,
    Empty,
    TooLarge,
}

impl Descriptor {
    pub const fn validate(self, dma: DmaWindow) -> Result<(), DescriptorError> {
        if self.length == 0 {
            return Err(DescriptorError::Empty);
        }
        if self.length > MAX_FRAME_BYTES {
            return Err(DescriptorError::TooLarge);
        }
        if self.address.checked_add(self.length as u64).is_none() {
            return Err(DescriptorError::AddressOverflow);
        }
        if !dma.contains(self.address, self.length as u64) {
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
            rx: status1 & (0xffff << interrupt::QUEUE_RX_SHIFT) != 0,
            tx: status1 & interrupt::QUEUE_TX_MASK != 0,
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

    #[test]
    fn dma_window_rejects_zero_and_wrapping_ranges() {
        assert_eq!(DmaWindow::new(0x1000, 0), None);
        assert_eq!(DmaWindow::new(u64::MAX, 2), None);
        assert!(DMA.contains(0x1000, 1));
        assert!(!DMA.contains(0x4fff, 2));
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
