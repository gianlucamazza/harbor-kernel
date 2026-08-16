//! BCM2711 GENET v5 control-plane, unpublished queue-0 program, PHY bring-up, bounded TX/RX, and reset.
//!
//! The verified FDT binding supplies the translated MMIO window and DMA
//! apertures. Packet ownership, ring arithmetic, and MDIO words stay in
//! `kernel_core::genet`. This layer writes those contracts into the
//! controller. Network-service publication is a later BSP composition step.

use kernel_core::genet::{
    self, DEFAULT_TX_RING, DESC_RING, Descriptor, DescriptorError, DmaPhase, LinkState, MdioError,
    MdioTxn, PhyError, PhyLink, QueueEnable, QueueEnableError, ResetReport, Revision,
    RevisionError, RgmiiReport, RingProgram, RingProgramError, RxReport, TbufReport, TxReport,
    UmacMibReport, UmacReport, dma_registers, mdio, phy, registers,
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
    tx_cpu: usize,
    tx_dma: u64,
    tx_len: u32,
    rx_cpu: usize,
    rx_dma: u64,
    rx_len: u32,
    queue: u8,
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
            tx_cpu: 0,
            tx_dma: 0,
            tx_len: 0,
            rx_cpu: 0,
            rx_dma: 0,
            rx_len: 0,
            queue: 0,
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
        self.tx_cpu = 0;
        self.tx_dma = 0;
        self.tx_len = 0;
        self.rx_cpu = 0;
        self.rx_dma = 0;
        self.rx_len = 0;
        self.queue = 0;
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
    /// descriptor. Descriptor addresses are device DMA addresses;
    /// `tx_cpu`/`rx_cpu` are the identity-mapped CPU addresses used for
    /// cache maintenance. Does not enable DMA and does not claim a live link.
    pub fn configure_queue0(
        &mut self,
        tx: Descriptor,
        rx: Descriptor,
        tx_cpu: usize,
        rx_cpu: usize,
    ) -> Result<(), Error> {
        self.configure_named_ring(DEFAULT_TX_RING, tx, rx, tx_cpu, rx_cpu)
    }

    /// Program the descriptor-based ring (16) on both engines. Not a NIC.
    pub fn configure_desc_ring(
        &mut self,
        tx: Descriptor,
        rx: Descriptor,
        tx_cpu: usize,
        rx_cpu: usize,
    ) -> Result<(), Error> {
        self.configure_named_ring(DESC_RING, tx, rx, tx_cpu, rx_cpu)
    }

    fn configure_named_ring(
        &mut self,
        queue: u8,
        tx: Descriptor,
        rx: Descriptor,
        tx_cpu: usize,
        rx_cpu: usize,
    ) -> Result<(), Error> {
        self.phase = self.phase.program().map_err(Error::Enable)?;
        tx.validate_windows(self.binding.dma)
            .map_err(Error::Descriptor)?;
        rx.validate_windows(self.binding.dma)
            .map_err(Error::Descriptor)?;
        let tx_ring = RingProgram::new(
            registers::TDMA,
            queue,
            0,
            1,
            tx.length
                .try_into()
                .map_err(|_| Error::Descriptor(DescriptorError::TooLarge))?,
        )
        .map_err(Error::Ring)?;
        let rx_ring = RingProgram::new(
            registers::RDMA,
            queue,
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
        self.tx_cpu = tx_cpu;
        self.tx_dma = tx.address;
        self.tx_len = tx.length;
        self.rx_cpu = rx_cpu;
        self.rx_dma = rx.address;
        self.rx_len = rx.length;
        self.queue = queue;
        // Descriptor RAM is Device MMIO. Packet buffers are Normal; clean TX
        // so the engine sees CPU stores, invalidate RX so stale lines cannot
        // shadow a later device write (ADR-0106).
        // SAFETY: the caller identity-mapped `tx_cpu`/`rx_cpu`; the DMA
        // addresses were already accepted by validate_windows.
        unsafe {
            cache::clean_dcache_poc(tx_cpu, tx.length as usize);
            cache::invalidate_dcache_poc(rx_cpu, rx.length as usize);
        }
        Ok(())
    }

    /// Enable programmed queue 0 on both engines. Refuses Idle and a second
    /// enable. Does not publish a network service.
    pub fn enable_queue0(&mut self) -> Result<(), Error> {
        self.enable_named_ring(DEFAULT_TX_RING)
    }

    /// Enable the programmed descriptor ring. Refuses unless it is ring 16.
    pub fn enable_desc_ring(&mut self) -> Result<(), Error> {
        self.enable_named_ring(DESC_RING)
    }

    fn enable_named_ring(&mut self, queue: u8) -> Result<(), Error> {
        if self.queue != queue {
            return Err(Error::Enable(QueueEnableError::UnsupportedQueue));
        }
        let enable = QueueEnable::new(queue).map_err(Error::Enable)?;
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

    fn ring_regs(&self, block: u32) -> usize {
        (block + dma_registers::RING_BASE + u32::from(self.queue) * dma_registers::RING_BYTES)
            as usize
    }

    /// One bounded TX on queue 0. Refuses unless Enabled and BMSR is up.
    /// Does not claim RX or publish a network service.
    pub fn submit_one_tx(&mut self) -> Result<TxReport, Error> {
        let link = self.classify_link()?;
        if let Some(refused) = TxReport::refuse(self.phase, link) {
            return Ok(refused);
        }
        if self.tx_cpu == 0 || self.tx_len < genet::TX_DMA_BYTES {
            return Ok(TxReport::NotEnabled);
        }
        let ring = self.ring_regs(registers::TDMA);
        if !TxReport::cons_is_idle(self.regs.read32(ring + dma_registers::CONS_INDEX as usize)) {
            return Ok(TxReport::ImplausibleCons);
        }
        if let Err(report) = self.program_umac_datapath() {
            return Ok(report);
        }
        self.assert_rgmii_link();
        self.pulse_tx_flush();
        fill_tsb_probe(self.tx_cpu, genet::TX_DMA_BYTES);
        // SAFETY: configure_queue0 stored an identity-mapped TX frame.
        unsafe {
            cache::clean_dcache_poc(self.tx_cpu, genet::TX_DMA_BYTES as usize);
        }
        let descriptor = Descriptor {
            address: self.tx_dma,
            length: genet::TX_DMA_BYTES,
            status: 0,
        };
        self.write_descriptor(registers::TDMA, 0, descriptor, true)?;
        self.regs
            .write32(ring + dma_registers::PROD_INDEX as usize, 1);
        if !poll::until(RESET_SPIN_LIMIT, || {
            TxReport::cons_has_posted(self.regs.read32(ring + dma_registers::CONS_INDEX as usize))
        }) {
            return Ok(TxReport::from_tx_cons(
                self.regs.read32(ring + dma_registers::CONS_INDEX as usize),
                genet::TX_DMA_BYTES as u16,
            ));
        }
        Ok(TxReport::from_tx_cons(
            self.regs.read32(ring + dma_registers::CONS_INDEX as usize),
            genet::TX_DMA_BYTES as u16,
        ))
    }

    /// One bounded RX on queue 0. Refuses unless Enabled and BMSR is up.
    /// Does not claim TX completion or publish a network service.
    pub fn submit_one_rx(&mut self) -> Result<RxReport, Error> {
        let link = self.classify_link()?;
        if let Some(refused) = RxReport::refuse(self.phase, link) {
            return Ok(refused);
        }
        if self.rx_cpu == 0 || self.rx_len == 0 {
            return Ok(RxReport::NotEnabled);
        }
        let ring = self.ring_regs(registers::RDMA);
        if !TxReport::cons_is_idle(self.regs.read32(ring + dma_registers::CONS_INDEX as usize)) {
            return Ok(RxReport::ImplausibleCons);
        }
        match self.program_umac_datapath() {
            Ok(()) => {}
            Err(TxReport::LinkDown) => return Ok(RxReport::LinkDown),
            Err(TxReport::UnknownSpeed) => return Ok(RxReport::UnknownSpeed),
            Err(TxReport::MdioTimeout) => return Ok(RxReport::MdioTimeout),
            Err(_) => return Ok(RxReport::NotEnabled),
        }
        self.assert_rgmii_link();
        // SAFETY: configure_queue0 stored an identity-mapped RX frame.
        unsafe {
            cache::invalidate_dcache_poc(self.rx_cpu, self.rx_len as usize);
        }
        let descriptor = Descriptor {
            address: self.rx_dma,
            length: self.rx_len,
            status: 0,
        };
        self.write_descriptor(registers::RDMA, 0, descriptor, true)?;
        let cmd = self.regs.read32(registers::UMAC_CMD as usize);
        self.regs.write32(
            registers::UMAC_CMD as usize,
            cmd | registers::UMAC_CMD_RX_EN,
        );
        self.regs
            .write32(ring + dma_registers::PROD_INDEX as usize, 1);
        if !poll::until(RESET_SPIN_LIMIT, || {
            let cons = self.regs.read32(ring + dma_registers::CONS_INDEX as usize);
            TxReport::cons_has_posted(cons)
                && matches!(
                    RxReport::from_status(self.regs.read32(registers::RDMA as usize)),
                    RxReport::Complete(_)
                )
        }) {
            return Ok(RxReport::from_poll(
                self.regs.read32(ring + dma_registers::CONS_INDEX as usize),
                self.regs.read32(registers::RDMA as usize),
            ));
        }
        // SAFETY: the device wrote the posted buffer; drop stale lines.
        unsafe {
            cache::invalidate_dcache_poc(self.rx_cpu, self.rx_len as usize);
        }
        Ok(RxReport::from_status(
            self.regs.read32(registers::RDMA as usize),
        ))
    }

    /// Write UniMAC speed and datapath enable bits before the doorbell.
    fn program_umac_datapath(&self) -> Result<(), TxReport> {
        let bmsr = self
            .mdio_read(phy::BMSR)
            .map_err(|_| TxReport::MdioTimeout)?;
        let lpa = self
            .mdio_read(phy::LPA)
            .map_err(|_| TxReport::MdioTimeout)?;
        let ctrl1000 = self
            .mdio_read(phy::CTRL1000)
            .map_err(|_| TxReport::MdioTimeout)?;
        let stat1000 = self
            .mdio_read(phy::STAT1000)
            .map_err(|_| TxReport::MdioTimeout)?;
        let speed = match genet::classify_aneg_speed(bmsr, lpa, ctrl1000, stat1000) {
            Ok(speed) => speed,
            Err(genet::SpeedError::LinkDown) => return Err(TxReport::LinkDown),
            Err(genet::SpeedError::Unknown) => return Err(TxReport::UnknownSpeed),
        };
        let cmd = self.regs.read32(registers::UMAC_CMD as usize);
        self.regs.write32(
            registers::UMAC_CMD as usize,
            genet::umac_cmd_datapath(cmd, speed),
        );
        Ok(())
    }

    /// Pulse `UMAC_TX_FLUSH`. The register has no completion bit.
    fn pulse_tx_flush(&self) {
        self.regs.write32(registers::UMAC_TX_FLUSH as usize, 1);
        let _ = self.regs.read32(registers::UMAC_TX_FLUSH as usize);
        self.regs.write32(registers::UMAC_TX_FLUSH as usize, 0);
        let _ = self.regs.read32(registers::UMAC_TX_FLUSH as usize);
    }

    /// UniMAC TX TSV candidates. Not a wire claim.
    pub fn read_umac_tsv(&self) -> UmacMibReport {
        UmacMibReport {
            packed: self.regs.read32(registers::UMAC_TX_PKTS_PACKED as usize),
            linux: self.regs.read32(registers::UMAC_TX_PKTS as usize),
            pok: self.regs.read32(registers::UMAC_TX_POK as usize),
        }
    }

    /// Enable TBUF 64-byte status-block mode. The probe frame carries a TSB.
    pub fn program_tbuf_tsb(&self) -> TbufReport {
        let current = self.regs.read32(registers::TBUF_CTRL as usize);
        self.regs
            .write32(registers::TBUF_CTRL as usize, genet::tbuf_with_tsb(current));
        self.regs
            .write32(registers::SYS_TBUF_FLUSH_CTRL as usize, 0);
        TbufReport::Tsb
    }

    /// Pulse then release UniMAC MIB reset so a later TSV read is not stuck at 0.
    fn release_umac_mib(&self) {
        self.regs.write32(
            registers::UMAC_MIB_CTRL as usize,
            genet::umac_mib_reset_bits(),
        );
        let _ = self.regs.read32(registers::UMAC_MIB_CTRL as usize);
        self.regs.write32(registers::UMAC_MIB_CTRL as usize, 0);
        let _ = self.regs.read32(registers::UMAC_MIB_CTRL as usize);
    }

    /// Program `SYS_PORT_CTRL` and the RGMII OOB block for `rgmii-rxid`.
    /// Does not claim link-up and does not publish a network service.
    pub fn program_rgmii_oob(&self) -> RgmiiReport {
        if !self.binding.phy_mode_rgmii_rxid {
            return RgmiiReport::ModeNotRgmiiRxid;
        }
        self.regs
            .write32(registers::SYS_PORT_CTRL as usize, genet::rgmii_port_ctrl());
        let current = self.regs.read32(registers::EXT_RGMII_OOB_CTRL as usize);
        self.regs.write32(
            registers::EXT_RGMII_OOB_CTRL as usize,
            genet::rgmii_oob_mode(current),
        );
        RgmiiReport::Programmed
    }

    /// Write UniMAC max-frame and the probe station address. Not a NIC.
    pub fn program_umac_init(&self) -> UmacReport {
        self.regs.write32(
            registers::UMAC_MAX_FRAME_LEN as usize,
            genet::MAX_FRAME_BYTES,
        );
        self.regs.write32(
            registers::UMAC_MAC0 as usize,
            genet::umac_mac0(genet::STATION_ADDR),
        );
        self.regs.write32(
            registers::UMAC_MAC1 as usize,
            genet::umac_mac1(genet::STATION_ADDR),
        );
        self.release_umac_mib();
        self.pulse_tx_flush();
        UmacReport::Programmed
    }

    /// OR `RGMII_LINK` after Enabled+Up. Does not change port mode.
    fn assert_rgmii_link(&self) {
        let current = self.regs.read32(registers::EXT_RGMII_OOB_CTRL as usize);
        self.regs.write32(
            registers::EXT_RGMII_OOB_CTRL as usize,
            genet::rgmii_oob_with_link(current),
        );
    }

    /// Stop DMA, UniMAC-reset, and return to Idle. Refuses Idle.
    /// Does not republish a network service.
    pub fn recover(&mut self) -> Result<ResetReport, Error> {
        if let Some(refused) = ResetReport::refuse(self.phase) {
            return Ok(refused);
        }
        self.reset()?;
        Ok(ResetReport::from_phase(self.phase))
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

    /// Clause-22 PHY identify only. Does not reset the PHY, classify BMSR,
    /// enable DMA, or publish a network service.
    pub fn identify_phy(&self) -> Result<PhyLink, Error> {
        if !self.binding.phy_mode_rgmii_rxid {
            return Err(Error::Phy(PhyError::ModeNotRgmiiRxid));
        }
        let hi = self.mdio_read(mdio::PHYIDR1)?;
        let lo = self.mdio_read(mdio::PHYIDR2)?;
        PhyLink::identify(hi, lo, true).map_err(Error::Phy)
    }

    /// BMSR snapshot only. Does not reset the PHY, require link-up,
    /// enable DMA, or publish a network service.
    pub fn classify_link(&self) -> Result<LinkState, Error> {
        let bmsr = self.mdio_read(phy::BMSR)?;
        Ok(PhyLink::classify_bmsr(bmsr))
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
        let length_status = if block == registers::TDMA {
            TxReport::tx_desc_status(words.length_status)
        } else {
            words.length_status
        };
        let offset = (block + u32::from(index) * genet::DESCRIPTOR_BYTES as u32) as usize;
        self.regs.write32(offset, length_status);
        self.regs.write32(offset + 4, words.address_low);
        self.regs.write32(offset + 8, words.address_high);
        Ok(())
    }
}

/// TSB prefix plus broadcast probe: dest ff:ff, src [`STATION_ADDR`], 0x88b5.
fn fill_tsb_probe(cpu: usize, len: u32) {
    let n = core::cmp::min(len as usize, genet::TX_DMA_BYTES as usize);
    // SAFETY: `cpu` is the identity-mapped TX frame stored by configure_queue0.
    let buf = unsafe { core::slice::from_raw_parts_mut(cpu as *mut u8, n) };
    genet::write_tsb_probe(buf);
}
