//! Deterministic GENET v5 device-model run (non-hardware evidence).
//!
//! This is intentionally not QEMU and does not claim a Pi 4 capture. It drives
//! the public FDT binding and ring ownership contract as a tiny virtual
//! device, so the host gate exercises the same TX/RX completion direction the
//! future MMIO backend must preserve.

use kernel_core::genet::{
    Descriptor, DescriptorStatus, Ownership, RingLayout, RingState, registers,
};
use kernel_core::genet_fdt;

const PI4: &[u8] = include_bytes!("../tests/fixtures/bcm2711-rpi-4-b.dtb");

#[test]
fn deterministic_genet_device_model_runs_bounded_tx_rx() {
    let binding = genet_fdt::extract(PI4).expect("Pi 4 DTB binding must be valid");
    let ring_dma = binding.dma.windows[0];
    let layout = RingLayout::new(registers::RDMA as u64, 2).unwrap();
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

    // Virtual TX device: driver posts, device clears OWN, driver reclaims.
    let mut tx = RingState::new(layout, binding.dma);
    assert_eq!(tx.post(descriptor), Ok(0));
    assert_eq!(tx.complete(completion).unwrap().0, 0);

    // Virtual RX device follows the same ownership path with a separate ring.
    let mut rx = RingState::new(layout, binding.dma);
    assert_eq!(rx.post(descriptor), Ok(0));
    let (_, received) = rx.complete(completion).unwrap();
    assert_eq!(received.address, packet);
    assert_eq!(received.length, 128);
}
