//! BCM2711 GENET v5 control-plane, unpublished queue-0 program, and PHY bring-up.
//!
//! The verified FDT binding supplies the translated MMIO window and DMA
//! apertures. Packet ownership, ring arithmetic, and MDIO words stay in
//! `kernel_core::genet`. This layer writes those contracts into the
//! controller. Network-service publication is a later BSP composition step.

use kernel_core::genet::{
    self, Descriptor, DescriptorError, DmaPhase, MdioError, MdioTxn, PhyError, PhyLink,
    QueueEnable, QueueEnableError, Revision, RevisionError, RingProgram, RingProgramError,
    dma_registers, mdio, phy, registers,
};
use kernel_core::genet_fdt::Binding;

use crate::arch::cache;
use crate::arch::mmio::Mmio;
use crate::arch::probe;
use kernel_core::poll;

const RESET_SPIN_LIMIT: u32 = 1_000_000;
const DMA_ENABLE_MASK: u32 = dma_registers::DMA_ENABLE | (1 << dma_registers::RING_BUF_EN_SHIFT);
const DMA_DISABLED: u32 = DMA_ENABLE_MASK;
const CMD_SW_RESET: u32 = 1 << 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidBinding,
    NotPresent,
    Revision(RevisionError),
    Timeout,
    Ring(RingProgramError),
    Descriptor(DescriptorError),
    Mdio(MdioError),
    Phy(PhyError),
    Enable(QueueEnableError),
}

/// A probed and reset GENET v5 controller.
pub struct Genet {
    regs: Mmio,
    binding: Binding,
    revision: Revision,
    phase: DmaPhase,
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
        let mut controller = Self {
            regs,
            binding,
            revision,
            phase: DmaPhase::Idle,
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

    pub const fn phase(&self) -> DmaPhase {
        self.phase
    }

    /// Return the controller to a quiescent, restartable state.
    pub fn reset(&mut self) -> Result<(), Error> {
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
        self.phase = self.phase.reset();
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
        QueueEnable { queue: 0 }.ctrl()
    }

    /// Program queue 0 on both DMA engines and publish one TX and one RX
    /// descriptor. Does not enable DMA and does not claim a live link.
    pub fn configure_queue0(&mut self, tx: Descriptor, rx: Descriptor) -> Result<(), Error> {
        self.phase = self.phase.program().map_err(Error::Enable)?;
        tx.validate_windows(self.binding.dma)
            .map_err(Error::Descriptor)?;
        rx.validate_windows(self.binding.dma)
            .map_err(Error::Descriptor)?;
        let tx_ring = RingProgram::new(
            registers::TDMA,
            0,
            0,
            1,
            tx.length
                .try_into()
                .map_err(|_| Error::Descriptor(DescriptorError::TooLarge))?,
        )
        .map_err(Error::Ring)?;
        let rx_ring = RingProgram::new(
            registers::RDMA,
            0,
            0,
            1,
            rx.length
                .try_into()
                .map_err(|_| Error::Descriptor(DescriptorError::TooLarge))?,
        )
        .map_err(Error::Ring)?;

        self.regs.write32(
            (registers::RDMA + dma_registers::SCB_BURST_SIZE) as usize,
            self.dma_burst_value(),
        );
        self.regs.write32(
            (registers::TDMA + dma_registers::SCB_BURST_SIZE) as usize,
            self.dma_burst_value(),
        );
        self.write_ring(tx_ring);
        self.write_ring(rx_ring);
        self.write_descriptor(registers::TDMA, 0, tx, true)?;
        self.write_descriptor(registers::RDMA, 0, rx, true)?;
        // Descriptor RAM is Device MMIO. Packet buffers are Normal; clean TX
        // so the engine sees CPU stores, invalidate RX so stale lines cannot
        // shadow a later device write (ADR-0106).
        // SAFETY: validate_windows accepted both buffers as DMA RAM.
        unsafe {
            cache::clean_dcache_poc(tx.address as usize, tx.length as usize);
            cache::invalidate_dcache_poc(rx.address as usize, rx.length as usize);
        }
        Ok(())
    }

    /// Enable programmed queue 0 on both engines. Refuses Idle and a second
    /// enable. Does not publish a network service.
    pub fn enable_queue0(&mut self) -> Result<(), Error> {
        let enable = QueueEnable::new(0).map_err(Error::Enable)?;
        self.phase = self.phase.enable().map_err(Error::Enable)?;
        for block in [registers::RDMA, registers::TDMA] {
            self.regs.write32(
                (block + dma_registers::RING_CFG) as usize,
                enable.ring_cfg(),
            );
            self.regs
                .write32((block + dma_registers::CTRL) as usize, enable.ctrl());
        }
        Ok(())
    }

    /// Clause-22 MDIO read of `reg` on the DT PHY address.
    pub fn mdio_read(&self, reg: u8) -> Result<u16, Error> {
        let phy = u8::try_from(self.binding.phy_addr)
            .map_err(|_| Error::Mdio(MdioError::PhyOutOfRange))?;
        let txn = MdioTxn::new(phy, reg, None).map_err(Error::Mdio)?;
        self.regs.write32(
            registers::UMAC_MDIO_CMD as usize,
            txn.encode().map_err(Error::Mdio)?,
        );
        if !poll::until(RESET_SPIN_LIMIT, || {
            self.regs.read32(registers::UMAC_MDIO_CMD as usize) & mdio::START_BUSY == 0
        }) {
            return Err(Error::Timeout);
        }
        MdioTxn::decode_read(self.regs.read32(registers::UMAC_MDIO_CMD as usize))
            .map_err(Error::Mdio)
    }

    /// Combine PHYIDR1/PHYIDR2. Absent or stuck-high IDs are a refusal.
    pub fn phy_id(&self) -> Result<u32, Error> {
        let hi = self.mdio_read(mdio::PHYIDR1)?;
        let lo = self.mdio_read(mdio::PHYIDR2)?;
        genet::classify_phy_id(hi, lo).map_err(Error::Mdio)
    }

    /// Identify the DT PHY, issue a bounded BMCR reset, and classify BMSR.
    ///
    /// Does not enable DMA and does not publish a network service.
    pub fn init_phy(&self) -> Result<PhyLink, Error> {
        if !self.binding.phy_mode_rgmii_rxid {
            return Err(Error::Phy(PhyError::ModeNotRgmiiRxid));
        }
        let hi = self.mdio_read(mdio::PHYIDR1)?;
        let lo = self.mdio_read(mdio::PHYIDR2)?;
        let identified = PhyLink::identify(hi, lo, true).map_err(Error::Phy)?;
        self.mdio_write(phy::BMCR, PhyLink::reset_command())?;
        if !poll::until(RESET_SPIN_LIMIT, || {
            self.mdio_read(phy::BMCR)
                .ok()
                .is_some_and(|bmcr| PhyLink::reset_cleared(bmcr).is_ok())
        }) {
            return Err(Error::Timeout);
        }
        let bmsr = self.mdio_read(phy::BMSR)?;
        identified.with_bmsr(bmsr).require_up().map_err(Error::Phy)
    }

    fn mdio_write(&self, reg: u8, data: u16) -> Result<(), Error> {
        let phy_addr = u8::try_from(self.binding.phy_addr)
            .map_err(|_| Error::Mdio(MdioError::PhyOutOfRange))?;
        let txn = MdioTxn::new(phy_addr, reg, Some(data)).map_err(Error::Mdio)?;
        self.regs.write32(
            registers::UMAC_MDIO_CMD as usize,
            txn.encode().map_err(Error::Mdio)?,
        );
        if !poll::until(RESET_SPIN_LIMIT, || {
            self.regs.read32(registers::UMAC_MDIO_CMD as usize) & mdio::START_BUSY == 0
        }) {
            return Err(Error::Timeout);
        }
        Ok(())
    }

    fn write_ring(&self, program: RingProgram) {
        let base = program.ring_register_base() as usize;
        let words = program.words();
        self.regs
            .write32(base + dma_registers::READ_PTR as usize, words.read_ptr);
        self.regs
            .write32(base + dma_registers::READ_PTR_HI as usize, 0);
        self.regs
            .write32(base + dma_registers::CONS_INDEX as usize, words.cons);
        self.regs
            .write32(base + dma_registers::PROD_INDEX as usize, words.prod);
        self.regs.write32(
            base + dma_registers::RING_BUF_SIZE as usize,
            words.ring_buf_size,
        );
        self.regs
            .write32(base + dma_registers::START_ADDR as usize, words.start);
        self.regs
            .write32(base + dma_registers::START_ADDR_HI as usize, words.start_hi);
        self.regs
            .write32(base + dma_registers::END_ADDR as usize, words.end);
        self.regs
            .write32(base + dma_registers::END_ADDR_HI as usize, words.end_hi);
        self.regs.write32(
            base + dma_registers::MBUF_DONE_THRESH as usize,
            words.mbuf_done,
        );
        self.regs
            .write32(base + dma_registers::FLOW_PERIOD as usize, words.flow);
        self.regs
            .write32(base + dma_registers::WRITE_PTR as usize, words.write_ptr);
        self.regs
            .write32(base + dma_registers::WRITE_PTR_HI as usize, 0);
    }

    fn write_descriptor(
        &self,
        block: u32,
        index: u16,
        descriptor: Descriptor,
        wrap: bool,
    ) -> Result<(), Error> {
        let words = descriptor
            .words(genet::Ownership::Device, true, true, wrap)
            .map_err(Error::Descriptor)?;
        let offset = (block + u32::from(index) * genet::DESCRIPTOR_BYTES as u32) as usize;
        self.regs.write32(offset, words.length_status);
        self.regs.write32(offset + 4, words.address_low);
        self.regs.write32(offset + 8, words.address_high);
        Ok(())
    }
}
