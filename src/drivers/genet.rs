//! BCM2711 GENET v5 control-plane driver.
//!
//! This layer owns only the device register lifecycle. The verified FDT
//! binding supplies the translated MMIO window and DMA apertures; packet
//! ownership and descriptor arithmetic remain in `kernel_core::genet`.
//! Network-service publication is intentionally a later BSP composition step.

use kernel_core::genet::{self, Revision, RevisionError};
use kernel_core::genet_fdt::Binding;

use crate::arch::mmio::Mmio;
use crate::arch::probe;
use kernel_core::poll;

const RESET_SPIN_LIMIT: u32 = 1_000_000;
const DMA_ENABLE_MASK: u32 = (1 << 0) | (1 << 1);
const DMA_ENABLE: u32 = 1 << 0;
const DMA_RING_BUF_EN_SHIFT: u32 = 1;
const DMA_DISABLED: u32 = DMA_ENABLE_MASK;
const CMD_SW_RESET: u32 = 1 << 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidBinding,
    NotPresent,
    Revision(RevisionError),
    Timeout,
}

/// A probed and reset GENET v5 controller.
pub struct Genet {
    regs: Mmio,
    binding: Binding,
    revision: Revision,
}

impl Genet {
    /// Probe the DT-described register window and establish the safe reset
    /// baseline: both interrupt blocks masked, both DMA engines stopped, and
    /// UniMAC in software reset.
    ///
    /// # Safety
    ///
    /// The binding must come from a complete, validated DTB; its MMIO window
    /// must be mapped Device-nGnRnE and exclusive to this controller.
    pub unsafe fn probe(binding: Binding) -> Result<Self, Error> {
        if binding.mmio_len != genet::REGISTER_BYTES
            || binding.interrupt_parent == 0
            || !binding.phy_mode_rgmii_rxid
            || binding.dma.count == 0
        {
            return Err(Error::InvalidBinding);
        }

        // SAFETY: the caller establishes that the DT-translated window is
        // mapped Device and is exclusively owned by this driver.
        let regs = unsafe { Mmio::new(binding.mmio_base as usize) };
        // SAFETY: the window is Device-mapped; an absent powered-down block is
        // converted from an external abort into a bounded refusal.
        let raw_revision =
            unsafe { probe::try_read32(regs.base() + genet::registers::SYS_REV_CTRL as usize) }
                .map_err(|_| Error::NotPresent)?;
        let revision = Revision::decode(raw_revision).map_err(Error::Revision)?;
        let controller = Self {
            regs,
            binding,
            revision,
        };
        controller.reset()?;
        Ok(controller)
    }

    pub const fn binding(&self) -> Binding {
        self.binding
    }

    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Mask and acknowledge both GENET interrupt blocks.
    pub fn mask_interrupts(&self) {
        for offset in [genet::registers::INTRL2_0, genet::registers::INTRL2_1] {
            self.regs.write32(
                (offset + genet::registers::INTRL2_CPU_MASK_SET) as usize,
                u32::MAX,
            );
            self.regs.write32(
                (offset + genet::registers::INTRL2_CPU_CLEAR) as usize,
                u32::MAX,
            );
        }
    }

    /// Return the controller to a quiescent, restartable state.
    pub fn reset(&self) -> Result<(), Error> {
        self.mask_interrupts();
        self.stop_dma(genet::registers::RDMA)?;
        self.stop_dma(genet::registers::TDMA)?;

        // Linux's v5 sequence clears the RBUF software-reset latch before
        // issuing UniMAC reset; the latch is deliberately not hidden behind a
        // delay-only assumption here.
        self.regs.write32(genet::registers::RBUF_CTRL as usize, 0);
        self.regs
            .write32(genet::registers::UMAC_CMD as usize, CMD_SW_RESET);
        if !poll::until(RESET_SPIN_LIMIT, || {
            self.regs.read32(genet::registers::UMAC_CMD as usize) & CMD_SW_RESET == 0
        }) {
            return Err(Error::Timeout);
        }
        Ok(())
    }

    fn stop_dma(&self, base: u32) -> Result<(), Error> {
        let ctrl = (base + genet::dma_registers::CTRL) as usize;
        let status = (base + genet::dma_registers::STATUS) as usize;
        let current = self.regs.read32(ctrl);
        self.regs.write32(ctrl, current & !DMA_ENABLE_MASK);
        if !poll::until(RESET_SPIN_LIMIT, || {
            self.regs.read32(status) & DMA_DISABLED == DMA_DISABLED
        }) {
            return Err(Error::Timeout);
        }
        Ok(())
    }

    /// Register value for the v5 burst policy selected by the BCM2711 DT
    /// binding. Kept as a method so the future DMA setup cannot silently pick
    /// a generic GENET default.
    pub const fn dma_burst_value(&self) -> u32 {
        genet::BCM2711_DMA_BURST
    }

    /// Queue-0 enable mask used by the first bounded TX/RX composition.
    pub const fn queue0_enable_mask() -> u32 {
        DMA_ENABLE | (1 << DMA_RING_BUF_EN_SHIFT)
    }
}
