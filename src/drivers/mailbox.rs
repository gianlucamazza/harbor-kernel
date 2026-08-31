//! Bounded VideoCore property-mailbox transport.
//!
//! The mailbox is an EL1-only firmware RPC. The validated result is returned
//! as data to the BSP; no mailbox register or request buffer is exposed to an
//! EL0 agent (ADR-0113).

use kernel_core::mailbox::{self, FramebufferInfo};

use crate::arch::{cache, mmio::Mmio};

const PROPERTY_CHANNEL: u32 = 8;
const READ: usize = 0x00;
const STATUS: usize = 0x18;
const WRITE: usize = 0x20;
const STATUS_FULL: u32 = 1 << 31;
const STATUS_EMPTY: u32 = 1 << 30;
const SPIN_LIMIT: u32 = 10_000_000;

#[repr(C, align(16))]
struct PropertyBuffer {
    words: [u32; mailbox::MAX_WORDS],
}

static mut PROPERTY_BUFFER: PropertyBuffer = PropertyBuffer {
    words: [0; mailbox::MAX_WORDS],
};

/// Why a property-mailbox framebuffer request failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    TimeoutWrite,
    TimeoutRead,
    WrongChannel(u32),
    AddressOutOfRange,
    Contract(mailbox::Error),
}

impl From<mailbox::Error> for Error {
    fn from(error: mailbox::Error) -> Self {
        Self::Contract(error)
    }
}

/// EL1 property-mailbox client for one exclusive BCM2711 mailbox block.
pub struct PropertyMailbox {
    regs: Mmio,
}

impl PropertyMailbox {
    /// Construct a client for the board-supplied mailbox register block.
    ///
    /// # Safety
    ///
    /// `base` must be the mapped BCM2711 mailbox block and no other caller may
    /// use its property channel while this client is active.
    pub unsafe fn new(base: usize) -> Self {
        Self {
            // SAFETY: the caller establishes the exclusive board MMIO window.
            regs: unsafe { Mmio::new(base) },
        }
    }

    /// Ask firmware to allocate and configure the product framebuffer.
    pub fn framebuffer(&self) -> Result<FramebufferInfo, Error> {
        let buffer = core::ptr::addr_of_mut!(PROPERTY_BUFFER);
        let address = buffer as usize;
        let address32 = u32::try_from(address).map_err(|_| Error::AddressOutOfRange)?;
        // SAFETY: this is the one static request buffer owned by this EL1
        // client; the request is not shared with any agent or other core.
        let words = unsafe { &mut (*buffer).words };
        let length = mailbox::build_request(words)?;
        // SAFETY: PROPERTY_BUFFER is Normal memory and the firmware reads it
        // through the mailbox bus after this clean-to-PoC operation.
        unsafe {
            cache::clean_dcache_poc(address, length * core::mem::size_of::<u32>());
        }

        self.write(address32)?;
        let response_channel = self.read_channel()?;
        if response_channel != PROPERTY_CHANNEL {
            return Err(Error::WrongChannel(response_channel));
        }
        // SAFETY: firmware has written the response into the Normal buffer;
        // invalidate before EL1 reads it.
        unsafe {
            cache::invalidate_dcache_poc(address, length * core::mem::size_of::<u32>());
        }
        mailbox::parse_response(words).map_err(Error::from)
    }

    fn write(&self, address: u32) -> Result<(), Error> {
        for _ in 0..SPIN_LIMIT {
            if self.regs.read32(STATUS) & STATUS_FULL == 0 {
                self.regs
                    .write32(WRITE, (address & !0xF) | PROPERTY_CHANNEL);
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::TimeoutWrite)
    }

    fn read_channel(&self) -> Result<u32, Error> {
        for _ in 0..SPIN_LIMIT {
            if self.regs.read32(STATUS) & STATUS_EMPTY == 0 {
                return Ok(self.regs.read32(READ) & 0xF);
            }
            core::hint::spin_loop();
        }
        Err(Error::TimeoutRead)
    }
}
