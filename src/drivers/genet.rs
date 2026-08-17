//! BCM2711 GENET v5 control-plane, unpublished queue-0 program, PHY bring-up, bounded TX/RX, and reset.
//!
//! The verified FDT binding supplies the translated MMIO window and DMA
//! apertures. Packet ownership, ring arithmetic, and MDIO words stay in
//! `kernel_core::genet`. This layer writes those contracts into the
//! controller. Network-service publication is a later BSP composition step.

use kernel_core::genet::{
    self, ArbiterReport, DEFAULT_TX_RING, DESC_RING, Descriptor, DescriptorError, DmaPhase,
    GenetBoot, HfbReport, LinkState, MdioError, MdioTxn, PhyError, PhyInitReport, PhyLink,
    PriorityReport, Queue0Report, QueueEnable, QueueEnableError, RbufChkReport, RbufReport,
    ResetReport, Revision, RevisionError, RgmiiReport, RingBufReport, RingCfgReport, RingProgram,
    RingProgramError, Rings14Report, RxReport, StateDump, TbufReport, TbufSizeReport, TxReport,
    TxRingSet, UmacMibReport, UmacReport, WrrPriority, dma_registers, mdio, phy, registers,
};
use kernel_core::genet_fdt::Binding;

use crate::arch::cache;
use crate::arch::mmio::Mmio;
use crate::arch::probe;
use crate::arch::timer;
use kernel_core::poll;

const RESET_SPIN_LIMIT: u32 = 1_000_000;
const CMD_SW_RESET: u32 = 1 << 13;

/// Linux's settle after a flush or reset-latch write (`udelay(10)`).
///
/// A register readback stood in for this and is not the same thing: the read
/// returns as soon as the bus does, which on Device-nGnRnE is nanoseconds. A
/// settle expressed as "one more transaction" is a settle of zero (ADR-0107).
const FLUSH_SETTLE_US: u32 = 10;
/// Linux's settle after asserting `CMD_SW_RESET` (`udelay(2)`).
const RESET_SETTLE_US: u32 = 2;

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

        // `bcmgenet_open`'s order: `bcmgenet_umac_reset` ("take MAC out of
        // reset") and then `init_umac`, whose first act is `reset_umac`
        // (`bcmgenet.c:3368-3370`).
        //
        // Both work on `SYS_RBUF_FLUSH_CTRL` in the **SYS** block, not on
        // `RBUF_CTRL` in the RBUF block. Linux reaches the two through one
        // helper name (`bcmgenet_rbuf_ctrl_get/set`, `:127-140`) that does not
        // say `SYS`, and Harbor had been zeroing the wrong one — so the latch
        // that holds UniMAC in reset was never touched at all.
        self.release_umac_reset();

        // `reset_umac` (`:2560-2571`): zero the latch, wait 10 µs, assert
        // `CMD_SW_RESET`, wait 2 µs — and **stop**. Linux does not wait for
        // that bit to clear and never clears it by hand.
        //
        // Harbor polled for it to self-clear and treated a timeout as a failed
        // probe. That poll used to pass while the MAC was held by the SYS
        // latch and `UMAC_CMD` read back as zero; with the latch released it
        // times out, which is the 2026-08-17 11:10 `probe unavailable
        // (Timeout)` and is evidence *for* the diagnosis, not against it. The
        // self-clearing contract was invented here; the state dump reports the
        // bit instead of a poll asserting it.
        self.regs
            .write32(genet::registers::SYS_RBUF_FLUSH_CTRL as usize, 0);
        settle(FLUSH_SETTLE_US);
        self.regs
            .write32(genet::registers::UMAC_CMD as usize, CMD_SW_RESET);
        settle(RESET_SETTLE_US);
        // …and then take the MAC back out of it. Linux leaves the bit asserted
        // and `umac_enable_set` guards on it; on BCM2711 it does not
        // self-clear, and the 11:15 state dump read `cmd=0x1002067` — reset
        // still held, with the whole datapath written over the top. A quiescent
        // controller is one that is out of reset with nothing enabled, not one
        // that is held.
        self.regs.write32(genet::registers::UMAC_CMD as usize, 0);
        settle(RESET_SETTLE_US);
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
        let mask = if base == genet::registers::TDMA {
            TxRingSet::V5.tdma_ctrl()
        } else {
            TxRingSet::V5.rdma_ctrl()
        };
        let current = self.regs.read32(ctrl);
        self.regs.write32(ctrl, current & !mask);
        if !poll::until(RESET_SPIN_LIMIT, || self.regs.read32(status) & mask == mask) {
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
    /// TDMA `RING_BUF_EN` is Linux `0x1f`.
    pub const fn queue0_enable_mask() -> u32 {
        TxRingSet::V5.tdma_ctrl()
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
        // Ring geometry is checked here so a bad one is refused before any
        // register is touched; the writes happen in `program_rings`, which
        // runs after UniMAC and after the flush prologue (ADR-0107 §1).
        self.ring_program(registers::TDMA, queue)?;
        self.ring_program(registers::RDMA, queue)?;
        self.tx_cpu = tx_cpu;
        self.tx_dma = tx.address;
        self.tx_len = tx.length;
        self.rx_cpu = rx_cpu;
        self.rx_dma = rx.address;
        self.rx_len = rx.length;
        self.queue = queue;
        Ok(())
    }

    /// Ring 0 owns Linux's `GENET_Q0_TX_BD_CNT` / `GENET_Q0_RX_BD_CNT`, and
    /// every ring's slot size is `RX_BUF_LENGTH` — not the packet length.
    ///
    /// Harbor programmed one BD with the frame length as the slot size, while
    /// `program_priority_tx_rings` placed rings 1–4 at BD 128 on the
    /// assumption ring 0 owned 0..127. The two disagreed
    /// (`bcmgenet.c:2730-2733`, `:2817-2819`, `:3022`).
    fn ring_program(&self, block: u32, queue: u8) -> Result<RingProgram, Error> {
        let count = match (block, queue) {
            (registers::TDMA, DEFAULT_TX_RING) => genet::V5_Q0_TX_BD_CNT,
            (registers::RDMA, DEFAULT_TX_RING) => genet::V5_Q0_RX_BD_CNT,
            _ => 1,
        };
        RingProgram::new(block, queue, 0, count, genet::RX_BUF_LENGTH).map_err(Error::Ring)
    }

    /// Write the SCB burst policy, ring 0 on both engines, and the two posted
    /// descriptors. Runs after UniMAC/RBUF/TBUF/HFB and after the flush, in
    /// Linux's `bcmgenet_init_dma` position; DMA is still disabled here.
    fn program_rings(&self) -> Result<(), Error> {
        self.regs.write32(
            (registers::RDMA + dma_registers::SCB_BURST_SIZE) as usize,
            self.dma_burst_value(),
        );
        self.regs.write32(
            (registers::TDMA + dma_registers::SCB_BURST_SIZE) as usize,
            self.dma_burst_value(),
        );
        self.write_ring(self.ring_program(registers::TDMA, self.queue)?);
        self.write_ring(self.ring_program(registers::RDMA, self.queue)?);
        let tx = Descriptor {
            address: self.tx_dma,
            length: self.tx_len,
            status: 0,
        };
        let rx = Descriptor {
            address: self.rx_dma,
            length: self.rx_len,
            status: 0,
        };
        self.write_descriptor(registers::TDMA, 0, tx)?;
        self.write_descriptor(registers::RDMA, 0, rx)?;
        // Descriptor RAM is Device MMIO. Packet buffers are Normal; clean TX
        // so the engine sees CPU stores, invalidate RX so stale lines cannot
        // shadow a later device write (ADR-0106).
        // SAFETY: the caller identity-mapped `tx_cpu`/`rx_cpu`; the DMA
        // addresses were already accepted by validate_windows.
        unsafe {
            cache::clean_dcache_poc(self.tx_cpu, self.tx_len as usize);
            cache::invalidate_dcache_poc(self.rx_cpu, self.rx_len as usize);
        }
        Ok(())
    }

    /// Linux `init_tx_queues` programs TDMA rings 1–4 (32 BDs from 128).
    /// Does not doorbell them. Not a NIC.
    pub fn program_priority_tx_rings(&self) -> Rings14Report {
        for queue in 1..=genet::V5_TX_QUEUES {
            let first = genet::v5_priority_tx_first(queue).expect("1..=4");
            let program = RingProgram::new(
                registers::TDMA,
                queue,
                first,
                genet::V5_TX_BDS_PER_Q,
                genet::RX_BUF_LENGTH,
            )
            .expect("priority TX ring");
            self.write_ring(program);
        }
        Rings14Report::Programmed
    }

    /// Linux `init_tx_queues` writes `DMA_ARBITER_WRR` before enable.
    pub fn program_tdma_wrr(&self) -> ArbiterReport {
        self.regs.write32(
            (registers::TDMA + dma_registers::ARB_CTRL) as usize,
            dma_registers::DMA_ARBITER_WRR,
        );
        ArbiterReport::Wrr
    }

    /// Linux `init_tx_queues` writes `DMA_PRIORITY_0/1/2` after the rings.
    pub fn program_tdma_priority(&self) -> PriorityReport {
        let words = WrrPriority::V5.words();
        self.regs.write32(
            (registers::TDMA + dma_registers::DMA_PRIORITY_0) as usize,
            words[0],
        );
        self.regs.write32(
            (registers::TDMA + dma_registers::DMA_PRIORITY_1) as usize,
            words[1],
        );
        self.regs.write32(
            (registers::TDMA + dma_registers::DMA_PRIORITY_2) as usize,
            words[2],
        );
        PriorityReport::Programmed
    }

    /// Enable programmed queue 0 on both engines. Reports come from the
    /// `RING_CFG` and TDMA `RING_BUF_EN` words just written. Refuses Idle
    /// and a second enable. Does not publish a network service.
    pub fn enable_queue0(&mut self) -> Result<(RingCfgReport, RingBufReport), Error> {
        self.enable_named_ring(DEFAULT_TX_RING)
    }

    /// Enable the programmed descriptor ring. Refuses unless it is ring 16.
    pub fn enable_desc_ring(&mut self) -> Result<(RingCfgReport, RingBufReport), Error> {
        self.enable_named_ring(DESC_RING)
    }

    fn enable_named_ring(&mut self, queue: u8) -> Result<(RingCfgReport, RingBufReport), Error> {
        if self.queue != queue {
            return Err(Error::Enable(QueueEnableError::UnsupportedQueue));
        }
        let enable = QueueEnable::new(queue).map_err(Error::Enable)?;
        self.phase = self.phase.enable().map_err(Error::Enable)?;
        self.regs.write32(
            (registers::RDMA + dma_registers::RING_CFG) as usize,
            enable.set.rdma_ring_cfg(),
        );
        self.regs.write32(
            (registers::TDMA + dma_registers::RING_CFG) as usize,
            enable.set.tdma_ring_cfg(),
        );
        self.regs.write32(
            (registers::RDMA + dma_registers::CTRL) as usize,
            enable.set.rdma_ctrl(),
        );
        self.regs.write32(
            (registers::TDMA + dma_registers::CTRL) as usize,
            enable.set.tdma_ctrl(),
        );
        Ok((RingCfgReport::Programmed, RingBufReport::Programmed))
    }

    /// Run the unpublished bring-up after the rings are programmed.
    /// `ring_cfg` comes from the enable write. Not a NIC.
    pub fn boot_after_program(&mut self, programmed: Queue0Report) -> GenetBoot {
        let mut boot = GenetBoot::after_program(programmed);
        if !matches!(programmed, Queue0Report::Programmed) {
            return boot;
        }
        // Linux order, and the whole point of this sequence (ADR-0107 §1):
        // `init_umac` — MIB, max frame, station, TBUF TSB, RBUF align/64B,
        // RBUF_CHK, RBUF_TBUF_SIZE — then the RGMII block, then `hfb_init`,
        // then `init_dma`'s flush prologue, then the rings, and `DMA_EN`
        // **last** (`bcmgenet.c:3351-3380`, `:3089-3180`). Harbor used to
        // enable DMA first and program UniMAC into a running engine.
        boot.umac = Some(self.program_umac_init());
        boot.tbuf = Some(self.program_tbuf_tsb());
        boot.tbuf_size = Some(self.program_rbuf_tbuf_size());
        boot.rbuf = Some(self.program_rbuf_64b());
        boot.rbuf_chk = Some(self.program_rbuf_chk());
        boot.rgmii = Some(self.program_rgmii_oob());
        boot.hfb = Some(self.clear_hfb());
        self.flush_before_rings();
        if self.program_rings().is_err() {
            boot.enabled = Some(Queue0Report::Enable(QueueEnableError::NotProgrammed));
            return boot;
        }
        boot.rings14 = Some(self.program_priority_tx_rings());
        boot.arb = Some(self.program_tdma_wrr());
        boot.prio = Some(self.program_tdma_priority());
        match self.enable_queue0() {
            Ok((cfg, buf)) => {
                boot.enabled = Some(Queue0Report::Enabled);
                boot.ring_cfg = Some(cfg);
                boot.ring_buf = Some(buf);
            }
            Err(Error::Enable(error)) => {
                boot.enabled = Some(Queue0Report::Enable(error));
                return boot;
            }
            Err(_) => {
                boot.enabled = Some(Queue0Report::Enable(QueueEnableError::NotProgrammed));
                return boot;
            }
        }
        boot.tx = Some(match self.submit_one_tx() {
            Ok(report) => report,
            Err(Error::Timeout) => TxReport::Timeout,
            Err(Error::Phy(PhyError::LinkDown)) => TxReport::LinkDown,
            Err(_) => TxReport::NotEnabled,
        });
        if boot.tx.is_some() {
            boot.mib = Some(self.read_umac_tsv());
        }
        boot.rx = Some(match self.submit_one_rx() {
            Ok(report) => report,
            Err(Error::Timeout) => RxReport::Timeout,
            Err(Error::Phy(PhyError::LinkDown)) => RxReport::LinkDown,
            Err(_) => RxReport::NotEnabled,
        });
        boot.state = Some(self.read_state());
        boot.recovered = Some(match self.recover() {
            Ok(report) => report,
            Err(Error::Timeout) => ResetReport::Timeout,
            Err(_) => ResetReport::NotEnabled,
        });
        boot
    }

    fn ring_regs(&self, block: u32) -> usize {
        (block + dma_registers::RING_BASE + u32::from(self.queue) * dma_registers::RING_BYTES)
            as usize
    }

    /// One bounded TX on queue 0. Refuses unless Enabled and BMSR is up
    /// at [`kernel_core::genet::LinkMoment::Submit`]. Does not claim RX
    /// or publish a network service.
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
        self.write_descriptor(registers::TDMA, 0, descriptor)?;
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
        self.write_descriptor(registers::RDMA, 0, descriptor)?;
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

    /// Linux `bcmgenet_init_dma`'s prologue: flush the TX queues and pulse the
    /// RBUF flush bit, both with a real 10 µs settle, **before** any ring is
    /// programmed and long before `DMA_EN` (`bcmgenet.c:3113-3123`).
    ///
    /// Harbor pulsed `TX_FLUSH` from `program_umac_init`, which ran after the
    /// DMA engines were already enabled, and used a register readback where
    /// Linux waits. Both are corrected here as one sequence claim (ADR-0107).
    fn flush_before_rings(&self) {
        self.regs.write32(registers::UMAC_TX_FLUSH as usize, 1);
        settle(FLUSH_SETTLE_US);
        self.regs.write32(registers::UMAC_TX_FLUSH as usize, 0);
        settle(FLUSH_SETTLE_US);

        let flush = self.regs.read32(registers::SYS_RBUF_FLUSH_CTRL as usize);
        self.regs.write32(
            registers::SYS_RBUF_FLUSH_CTRL as usize,
            flush | registers::SYS_RBUF_FLUSH,
        );
        settle(FLUSH_SETTLE_US);
        self.regs
            .write32(registers::SYS_RBUF_FLUSH_CTRL as usize, flush);
        settle(FLUSH_SETTLE_US);
    }

    /// Linux `bcmgenet_umac_reset`, commented *"take MAC out of reset"* and
    /// called from `bcmgenet_open` immediately before `init_umac`
    /// (`bcmgenet.c:3299-3311`, `:3368`): pulse `SYS_UMAC_SW_RESET` in
    /// `SYS_RBUF_FLUSH_CTRL`, 10 µs on each edge.
    ///
    /// Harbor had no analogue. `umac_enable_set` refuses to write `UMAC_CMD`
    /// while that bit is set (`:2540-2545`), which is the exact shape of what
    /// silicon has shown for twenty-six boots: TDMA retires the descriptor and
    /// UniMAC counts nothing.
    fn release_umac_reset(&self) {
        let flush = self.regs.read32(registers::SYS_RBUF_FLUSH_CTRL as usize);
        self.regs.write32(
            registers::SYS_RBUF_FLUSH_CTRL as usize,
            flush | registers::SYS_UMAC_SW_RESET,
        );
        settle(FLUSH_SETTLE_US);
        self.regs.write32(
            registers::SYS_RBUF_FLUSH_CTRL as usize,
            flush & !registers::SYS_UMAC_SW_RESET,
        );
        settle(FLUSH_SETTLE_US);
    }

    /// Read back what the controller holds. Writes nothing (ADR-0107 §4).
    pub fn read_state(&self) -> StateDump {
        StateDump {
            sys_rbuf_flush: self.regs.read32(registers::SYS_RBUF_FLUSH_CTRL as usize),
            sys_port_ctrl: self.regs.read32(registers::SYS_PORT_CTRL as usize),
            rgmii_oob: self.regs.read32(registers::EXT_RGMII_OOB_CTRL as usize),
            umac_cmd: self.regs.read32(registers::UMAC_CMD as usize),
            rbuf_ctrl: self.regs.read32(registers::RBUF_CTRL as usize),
            tbuf_ctrl: self.regs.read32(registers::TBUF_CTRL as usize),
            tdma_ctrl: self
                .regs
                .read32((registers::TDMA + dma_registers::CTRL) as usize),
            tdma_status: self
                .regs
                .read32((registers::TDMA + dma_registers::STATUS) as usize),
            rdma_ctrl: self
                .regs
                .read32((registers::RDMA + dma_registers::CTRL) as usize),
            rdma_status: self
                .regs
                .read32((registers::RDMA + dma_registers::STATUS) as usize),
            tx_desc_status: self.regs.read32(registers::TDMA as usize),
        }
    }

    /// Linux `bcmgenet_hfb_clear`: disable the hardware filter block, zero
    /// every filter, zero the eight RDMA index-to-ring words, then re-enable
    /// filter 0 with length 4 — the "default flow to ring 0"
    /// (`bcmgenet.c:720-741`).
    ///
    /// Harbor never touched HFB. On a board whose firmware brings GENET up for
    /// network boot, that means inheriting whatever filtering the firmware
    /// left, and `HFB_CTRL = 0` is what Linux calls *disable MAC receive*.
    /// Not an RX claim: this only removes a reason RX could never arrive.
    pub fn clear_hfb(&self) -> HfbReport {
        self.regs.write32(registers::HFB_CTRL as usize, 0);
        self.regs.write32(registers::HFB_FLT_ENABLE as usize, 0);
        self.regs
            .write32((registers::HFB_FLT_ENABLE + 4) as usize, 0);
        for word in 0..dma_registers::INDEX2RING_COUNT {
            self.regs.write32(
                (registers::RDMA + dma_registers::INDEX2RING_0 + 4 * word) as usize,
                0,
            );
        }

        for filter in 0..genet::HFB_FILTER_COUNT {
            self.set_hfb_filter_length(filter, 0);
            for word in 0..genet::HFB_FILTER_WORDS {
                if let Some(offset) = genet::hfb_filter_ram_offset(filter, word) {
                    self.regs.write32(offset as usize, 0);
                }
            }
        }

        self.set_hfb_filter_length(0, genet::HFB_DEFAULT_FLOW_LEN);
        self.enable_hfb_filter(0);
        HfbReport::Cleared
    }

    fn set_hfb_filter_length(&self, filter: u32, length: u32) {
        let Some(offset) = genet::hfb_filter_length_offset(filter) else {
            return;
        };
        let current = self.regs.read32(offset as usize);
        self.regs.write32(
            offset as usize,
            genet::hfb_filter_length_word(current, filter, length),
        );
    }

    fn enable_hfb_filter(&self, filter: u32) {
        let Some(offset) = genet::hfb_filter_enable_offset(filter) else {
            return;
        };
        let current = self.regs.read32(offset as usize);
        self.regs.write32(
            offset as usize,
            current | genet::hfb_filter_enable_bit(filter),
        );
        let ctrl = self.regs.read32(registers::HFB_CTRL as usize);
        self.regs
            .write32(registers::HFB_CTRL as usize, ctrl | registers::HFB_EN);
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

    /// Linux v3+ `init_umac` writes `1` here. Not a NIC.
    pub fn program_rbuf_tbuf_size(&self) -> TbufSizeReport {
        self.regs.write32(
            registers::RBUF_TBUF_SIZE_CTRL as usize,
            registers::RBUF_TBUF_SIZE,
        );
        TbufSizeReport::Programmed
    }

    /// Linux `init_umac` ORs `RBUF_ALIGN_2B | RBUF_64B_EN`. Not a NIC.
    pub fn program_rbuf_64b(&self) -> RbufReport {
        let current = self.regs.read32(registers::RBUF_CTRL as usize);
        self.regs.write32(
            registers::RBUF_CTRL as usize,
            genet::rbuf_ctrl_with_64b_align(current),
        );
        RbufReport::Programmed
    }

    /// Linux `init_umac` writes `RBUF_CHK_CTRL`. Harbor datapath has `CRC_FWD`.
    pub fn program_rbuf_chk(&self) -> RbufChkReport {
        let current = self.regs.read32(registers::RBUF_CHK_CTRL as usize);
        self.regs.write32(
            registers::RBUF_CHK_CTRL as usize,
            genet::rbuf_chk_ctrl(current, true),
        );
        RbufChkReport::Programmed
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

    /// Bounded BMCR reset. Does not classify BMSR, require link-up, enable
    /// DMA, or publish a network service.
    pub fn reset_phy(&self) -> Result<PhyInitReport, Error> {
        if !self.binding.phy_mode_rgmii_rxid {
            return Err(Error::Phy(PhyError::ModeNotRgmiiRxid));
        }
        self.mdio_write(phy::BMCR, PhyLink::reset_command())?;
        if !poll::until(RESET_SPIN_LIMIT, || {
            self.mdio_read(phy::BMCR)
                .ok()
                .is_some_and(|bmcr| PhyLink::reset_cleared(bmcr).is_ok())
        }) {
            return Err(Error::Timeout);
        }
        Ok(PhyInitReport::Reset)
    }

    /// Identify the DT PHY, issue a bounded BMCR reset, and classify BMSR.
    ///
    /// Does not enable DMA and does not publish a network service.
    pub fn init_phy(&self) -> Result<PhyLink, Error> {
        let identified = self.identify_phy()?;
        self.reset_phy()?;
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

    /// Write one descriptor, with the words Linux writes and no others.
    ///
    /// TX (`bcmgenet_xmit`, `bcmgenet.c:2184-2200`): length, `SOP`, `EOP`,
    /// `APPEND_CRC` and the v5 qtag. **No `DMA_OWN`, no `DMA_WRAP`** — Harbor
    /// set both, and already knew the silicon never writes `OWN` back on TX
    /// (`kernel_core::genet::TxReport::from_tx_cons`).
    ///
    /// RX (`bcmgenet_rx_refill`, `bcmgenet.c:2261`): the address **only**.
    /// Linux never writes a length/status word for an RX buffer; the device
    /// owns that word and Harbor was overwriting it with a driver-invented
    /// one before every submit.
    fn write_descriptor(
        &self,
        block: u32,
        index: u16,
        descriptor: Descriptor,
    ) -> Result<(), Error> {
        let words = descriptor
            .words(genet::Ownership::Driver, true, true, false)
            .map_err(Error::Descriptor)?;
        let offset = (block + u32::from(index) * genet::DESCRIPTOR_BYTES as u32) as usize;
        if block == registers::TDMA {
            self.regs
                .write32(offset, TxReport::tx_desc_status(words.length_status));
        }
        self.regs.write32(offset + 4, words.address_low);
        self.regs.write32(offset + 8, words.address_high);
        Ok(())
    }
}

/// Wall-time settle between two register writes that the controller needs to
/// see separated. Backed by `CNTFRQ_EL0`, not by a spin count: the laptop's
/// spin budget and the Pi's are different numbers for the same microsecond.
fn settle(us: u32) {
    timer::busy_wait_us(us);
}

/// TSB prefix plus broadcast probe: dest ff:ff, src [`STATION_ADDR`], 0x88b5.
fn fill_tsb_probe(cpu: usize, len: u32) {
    let n = core::cmp::min(len as usize, genet::TX_DMA_BYTES as usize);
    // SAFETY: `cpu` is the identity-mapped TX frame stored by configure_queue0.
    let buf = unsafe { core::slice::from_raw_parts_mut(cpu as *mut u8, n) };
    genet::write_tsb_probe(buf);
}
