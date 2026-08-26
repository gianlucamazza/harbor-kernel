//! Absent-NIC transport for the q35 lab board.
//!
//! The lab image (ADR-0071) is a banner-and-halt composition with no NIC
//! window compiled, and QEMU's q35 exposes its virtio devices over PCI, not
//! virtio-mmio. The honest backend here is therefore the absent-device
//! refusal itself: [`claim`] succeeds — the board was surveyed and has no mmio
//! NIC slot to offer — and every device operation refuses, exactly as the
//! product boards do when their FDT or aperture comes up empty (ADR-0105).
//! This keeps `board::net` total across all three boards without teaching the
//! lab board a device it does not have.

use kernel_core::net::PacketToken;

use crate::bsp::net::{Buffer, Completion, RxSlot, StartFacts};

pub type Backend = Missing;

/// The backend keeps no device structures of its own.
pub const SCRATCH_PAGES: usize = 6;

/// Why the absent device refused an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No NIC slot exists on this board to claim or drive.
    NotPresent,
}

/// The surveyed-absent backend. Constructible, stateless, and always refusing:
/// the same shape a real backend shows when its device is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Missing;

/// Survey the board's (empty) NIC inventory.
pub fn claim(_scratch: &[u64; SCRATCH_PAGES]) -> Result<(Missing, StartFacts), Error> {
    Ok((
        Missing,
        StartFacts {
            base: 0,
            identity: 0,
            features: 0,
            queues: 0,
            queue_size: 0,
        },
    ))
}

impl crate::bsp::net::Transport for Missing {
    type Error = Error;

    fn start(&mut self, _rx: &[RxSlot], _tx: Buffer) -> Result<(), Self::Error> {
        Err(Error::NotPresent)
    }

    fn submit_tx(&mut self, _token: PacketToken, _buffer: Buffer) -> Result<(), Self::Error> {
        Err(Error::NotPresent)
    }

    fn return_rx(&mut self, _token: PacketToken, _buffer: Buffer) -> Result<(), Self::Error> {
        Err(Error::NotPresent)
    }

    fn take_tx_complete(&mut self) -> Option<(PacketToken, Completion)> {
        None
    }

    fn take_rx_available(&mut self) -> Option<(PacketToken, Completion)> {
        None
    }

    fn poll(&mut self) -> Result<(), Self::Error> {
        // Nothing to advance: the survey found no device to poll.
        Ok(())
    }

    fn reset(&mut self) -> Result<(), Self::Error> {
        Err(Error::NotPresent)
    }
}
