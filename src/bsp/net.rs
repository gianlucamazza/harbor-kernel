//! Board-neutral contract for the resident packet transport.
//!
//! The active board re-exports one implementation from [`crate::bsp::board::net`]
//! (ADR-0112 §2): a fixed seven-operation [`Transport`] surface, selected by
//! `cfg` at build time — the same shape as every other board difference in
//! this kernel. Not a `dyn Transport` (no allocator on this path, no indirect
//! call in the packet path) and not a generic parameter (one board builds one
//! kernel).
//!
//! Ownership split (ADR-0112 §2): `network_runtime` keeps the pool, the frame
//! ownership and the generation counter — those are service state, identical
//! for both backends — and delegates only the device half. This module is that
//! device half's contract. It deliberately carries physical buffers, not
//! packet-pool policy.

use kernel_core::net::PacketToken;

/// Whole pages of identity-mapped Normal memory this board's backend consumes
/// for its own device structures (queue rings and their shadows).
///
/// Bootstrap allocates them because the driver and BSP layers must not import
/// the allocator; the frames stay owned by `network_runtime`, which frees them
/// when the lease drops. A backend whose structures live in device memory
/// declares zero and ignores the slice.
pub use crate::bsp::board::net::SCRATCH_PAGES;

/// Why the board's NIC could not be claimed or driven.
pub type Error = <Backend as Transport>::Error;

/// The board-selected backend type behind the transport boundary.
///
/// One concrete struct per board, always reachable as `board::net::Backend`.
pub type Backend = crate::bsp::board::net::Backend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buffer {
    /// Identity-mapped CPU physical address of the Ethernet frame bytes.
    ///
    /// Device DMA addresses are the backend's business: the virtio backend
    /// posts this address unchanged, the GENET backend translates it through
    /// its FDT DMA windows before writing a descriptor.
    pub pa: u64,
    /// Frame capacity (RX) or frame length (TX), in bytes.
    ///
    /// Buffers carry a complete Ethernet L2 frame. Backends add and remove
    /// only their private framing (virtio-net's 12-byte header or GENET's
    /// 64-byte TSB) inside the same page.
    pub len: usize,
}

/// One receive buffer plus the opaque identity to hand back when it fills.
///
/// `start` publishes the initial receive set with these identities; every
/// later repost of the same buffer travels through
/// [`Transport::return_rx`](Transport::return_rx) carrying the token the
/// service currently recognises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RxSlot {
    pub token: PacketToken,
    pub buffer: Buffer,
}

/// What the device reported when it finished with one posted buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Completion {
    /// Backend-local slot index the buffer occupied (diagnostic).
    pub buffer: u16,
    /// Ethernet frame bytes now beginning at the buffer's physical address.
    pub len: usize,
}

/// Device facts observed while claiming, for the boot transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartFacts {
    /// Device register window base (CPU physical address).
    pub base: u64,
    /// Device identity word: virtio vendor id, or the GENET SYS_REV_CTRL
    /// revision word shape (major<<24 | minor<<16 | patch).
    pub identity: u32,
    /// Feature word the backend enabled (negotiated bits, ring masks, …).
    pub features: u64,
    pub queues: usize,
    pub queue_size: usize,
}

/// The seven operations needed by the transport-neutral network service
/// (ADR-0112 §2). Semantics each implementation owes:
///
/// - [`start`](Transport::start) claims the device, publishes every RX slot,
///   and puts one bring-up probe frame built into `tx` on the wire — or
///   refuses, leaving the buffers untouched.
/// - Buffers handed across are complete Ethernet frames (see [`Buffer`]); on
///   RX delivery the frame has been compacted to begin at the buffer address.
/// - Tokens are opaque identities: the backend stores what it was given and
///   hands the same value back with the matching completion.
/// - [`reset`](Transport::reset) restarts the device and drops TX/RX state;
///   the runtime republishes fresh receive identities after resetting its
///   packet pool, and tokens from before the reset are never reported again.
pub trait Transport {
    type Error: Copy;

    fn start(&mut self, rx: &[RxSlot], tx: Buffer) -> Result<(), Self::Error>;
    fn submit_tx(&mut self, token: PacketToken, buffer: Buffer) -> Result<(), Self::Error>;
    fn return_rx(&mut self, token: PacketToken, buffer: Buffer) -> Result<(), Self::Error>;
    fn take_tx_complete(&mut self) -> Option<(PacketToken, Completion)>;
    fn take_rx_available(&mut self) -> Option<(PacketToken, Completion)>;
    fn poll(&mut self) -> Result<(), Self::Error>;
    fn reset(&mut self) -> Result<(), Self::Error>;
}

/// Claim the board's NIC half on behalf of the resident service.
///
/// Identifies the device; nothing is transmitted and no buffer is published
/// until [`Transport::start`]. An absent device is a claimed refusal — `Err`,
/// not a panic — which is how a NIC-less boot stays honest (ADR-0105: the
/// refusal path carries hardware evidence).
///
/// Safe because each board discharges its own mapping invariants internally:
/// apertures it names are mapped Device in its compiled region table, and
/// `scratch` pages arrive from bootstrap's frame pool exclusively owned,
/// zeroed, and identity-mapped.
pub fn claim(scratch: &[u64; SCRATCH_PAGES]) -> Result<(Backend, StartFacts), Error> {
    crate::bsp::board::net::claim(scratch)
}
