//! Deterministic GENET v5 device-model run (non-hardware evidence).
//!
//! This is intentionally not QEMU and does not claim a Pi 4 capture. It drives
//! the public FDT binding and ring ownership contract as a tiny virtual
//! device, so the host gate exercises the same TX/RX completion direction the
//! future MMIO backend must preserve.

use kernel_core::genet::{
    Descriptor, DescriptorStatus, DmaPhase, LinkState, MdioError, MdioTxn, Ownership, PhyError,
    PhyLink, QueueEnable, RingLayout, RingProgram, RingState, classify_phy_id, dma_registers, mdio,
    phy, registers,
};
use kernel_core::genet_fdt;

const PI4: &[u8] = include_bytes!("../tests/fixtures/bcm2711-rpi-4-b.dtb");

#[test]
fn deterministic_genet_device_model_runs_bounded_tx_rx() {
    let binding = genet_fdt::extract(PI4).expect("Pi 4 DTB binding must be valid");
    let ring_dma = binding.dma.windows[0];
    let packet = ring_dma.base + 0x2000;
    let descriptor = Descriptor {
        address: packet,
        length: 128,
        status: 0,
    };
    let completion = DescriptorStatus {
        length: 128,
        ownership: Ownership::Driver,
        start: true,
        end: true,
        wrap: false,
    }
    .encode()
    .unwrap();

    let tx_program = RingProgram::new(registers::TDMA, 0, 0, 1, 128).unwrap();
    let rx_program = RingProgram::new(registers::RDMA, 0, 0, 1, 128).unwrap();
    assert_eq!(tx_program.ring_register_base(), registers::TDMA + 0xc00);
    assert_eq!(rx_program.ring_register_base(), registers::RDMA + 0xc00);
    assert_ne!(u64::from(tx_program.start_words()), packet);
    assert_eq!(tx_program.words().ring_buf_size, 1 << 16 | 128);

    let tx_layout = RingLayout::new(registers::TDMA as u64, 1).unwrap();
    let rx_layout = RingLayout::new(registers::RDMA as u64, 1).unwrap();
    assert_eq!(
        tx_layout.descriptor_address(0),
        Some(registers::TDMA as u64)
    );
    assert_eq!(
        tx_layout.descriptor_address(0).unwrap() + dma_registers::DESCRIPTOR_RAM_BYTES as u64,
        tx_program.ring_register_base() as u64
    );

    // Virtual TX device: driver posts, device clears OWN, driver reclaims.
    let mut tx = RingState::new(tx_layout, binding.dma);
    assert_eq!(tx.post(descriptor), Ok(0));
    assert_eq!(tx.complete(completion).unwrap().0, 0);

    // Virtual RX device follows the same ownership path with a separate ring.
    let mut rx = RingState::new(rx_layout, binding.dma);
    assert_eq!(rx.post(descriptor), Ok(0));
    let (_, received) = rx.complete(completion).unwrap();
    assert_eq!(received.address, packet);
    assert_eq!(received.length, 128);

    let cmd = MdioTxn::new(binding.phy_addr as u8, mdio::PHYIDR1, None)
        .unwrap()
        .encode()
        .unwrap();
    assert_eq!((cmd >> mdio::PHY_SHIFT) & mdio::PHY_MASK, binding.phy_addr);
    assert_eq!(
        MdioTxn::decode_read(cmd),
        Err(kernel_core::genet::MdioError::Busy)
    );
    // Device-model reply: START_BUSY cleared, identifier in the data field.
    assert_eq!(MdioTxn::decode_read(0x0362), Ok(0x0362));
    assert_eq!(classify_phy_id(0x0362, 0x5e60), Ok(0x0362_5e60));
}

#[test]
fn deterministic_phy_bring_up_and_absent_id() {
    let binding = genet_fdt::extract(PI4).expect("Pi 4 DTB binding must be valid");
    assert!(binding.phy_mode_rgmii_rxid);

    let link = PhyLink::identify(0x0362, 0x5e60, binding.phy_mode_rgmii_rxid).unwrap();
    let reset = MdioTxn::new(
        binding.phy_addr as u8,
        phy::BMCR,
        Some(PhyLink::reset_command()),
    )
    .unwrap()
    .encode()
    .unwrap();
    assert_ne!(reset & mdio::WRITE, 0);
    assert_eq!(
        PhyLink::reset_cleared(phy::BMCR_RESET),
        Err(PhyError::ResetPending)
    );
    assert_eq!(PhyLink::reset_cleared(0), Ok(()));
    assert_eq!(
        link.with_bmsr(phy::BMSR_LINK | phy::BMSR_ANEG_DONE)
            .require_up()
            .unwrap()
            .state,
        LinkState::Up
    );

    assert_eq!(
        PhyLink::identify(0, 0, true),
        Err(PhyError::Id(MdioError::AbsentPhyId))
    );
    assert_eq!(
        PhyLink::identify(0xffff, 0xffff, true),
        Err(PhyError::Id(MdioError::StuckHighPhyId))
    );

    let enable = QueueEnable::new(0).unwrap();
    assert_eq!(enable.ring_cfg(), 1);
    assert_eq!(enable.tdma_ring_cfg(), 0x1f);
    assert_eq!(enable.ctrl(), dma_registers::DMA_ENABLE | (1 << 1));
    assert_eq!(
        DmaPhase::Idle.enable(),
        Err(kernel_core::genet::QueueEnableError::NotProgrammed)
    );
    assert_eq!(
        DmaPhase::Idle.program().unwrap().enable(),
        Ok(DmaPhase::Enabled)
    );
}
