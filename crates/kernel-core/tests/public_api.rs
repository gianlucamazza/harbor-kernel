//! The crate as the kernel sees it: from outside, through the public API only.
//!
//! Every other test in this crate lives beside the code it exercises and can
//! reach private state. That is the right shape for checking arithmetic, and it
//! cannot see a different class of regression: a type that stops being
//! exported, a method that becomes private, a constructor that grows a
//! parameter, an enum that loses a variant a caller matches on. Those compile
//! fine inside the module and break `src/`.
//!
//! So these tests do what `src/` does. They import through `kernel_core::…`,
//! instantiate with the same const parameters the BSP uses, and assert on the
//! behaviour the kernel actually depends on — not on internals, which they
//! cannot see either.

use kernel_core::cap::{CapId, CapRights};
use kernel_core::ipc::{Message, RecvError, SendError, Table};
use kernel_core::layout::{UserWindow, WindowError};
use kernel_core::paging::{Level, MemKind, Perms};
use kernel_core::runqueue::TaskId;
use kernel_core::tasks::{Decision, Switch, Tasks};

/// The shapes `src/` instantiates. If a const parameter is reordered or a type
/// stops being exported, this file stops compiling — which is the point.
type KernelIpc = Table<8, 16, 4>;
type KernelTasks = Tasks<12>;

const KERNEL_WINDOW: UserWindow = UserWindow {
    base: 0x0000_0000_4000_0000,
    pages: 4,
    text_pages: 1,
    frame: 0x1000,
};

#[test]
fn a_channel_carries_a_message_between_two_holders() {
    let mut ipc = KernelIpc::new();
    let ch = ipc.create_channel().expect("a fresh table has room");
    let msg = Message {
        tag: 7,
        a: 0xDEAD,
        b: 0xBEEF,
    };

    assert_eq!(ipc.send(ch.send, msg), Ok(None));
    assert_eq!(ipc.try_recv(ch.recv), Ok(msg));
}

#[test]
fn the_authority_count_is_reachable_from_outside() {
    // `bootstrap` prints this number and the boot check asserts on it, so it
    // has to be readable without touching internals.
    let mut ipc = KernelIpc::new();
    assert_eq!(ipc.refusals().authority, 0);

    let forged = CapId::new(4, 999);
    assert_eq!(
        ipc.send(forged, Message { tag: 0, a: 0, b: 0 }),
        Err(SendError::BadCap)
    );
    assert_eq!(ipc.refusals().authority, 1);
    assert_eq!(ipc.refusals().full, 0, "and not the other two");
    assert_eq!(ipc.refusals().state, 0);
}

#[test]
fn a_receiver_parks_and_a_sender_names_it() {
    // The sequence `src/ipc::recv` performs: try, park, and let the send report
    // who to wake. Written from outside because that ordering is the contract.
    let mut ipc = KernelIpc::new();
    let ch = ipc.create_channel().unwrap();
    let me = TaskId::new(5, 0);

    assert_eq!(ipc.try_recv(ch.recv), Err(RecvError::Empty));
    assert_eq!(ipc.park(ch.recv, me), Ok(None));
    assert_eq!(
        ipc.send(ch.send, Message { tag: 1, a: 0, b: 0 }),
        Ok(Some(me))
    );
}

#[test]
fn the_scheduler_hands_back_an_exited_task_stack() {
    // What `src/sched` needs: a decision it can act on, and a way to learn
    // whose stack is now free.
    let mut tasks = KernelTasks::new();
    tasks.start();
    let worker = tasks.admit().expect("eleven slots besides idle");

    assert!(matches!(
        tasks.switch(Switch::Yield),
        Decision::Switch { to, .. } if to == worker
    ));
    assert!(matches!(
        tasks.switch(Switch::Exit),
        Decision::Switch { .. }
    ));
    assert_eq!(tasks.collect(), Some(worker));
    assert_eq!(tasks.overwrites(), 0);
}

#[test]
fn the_user_window_refuses_a_write_past_its_text_page() {
    assert_eq!(KERNEL_WINDOW.entry(), 0x4000_0000);
    assert_eq!(KERNEL_WINDOW.stack_top(), 0x4000_4000);
    assert_eq!(KERNEL_WINDOW.bound_text_write(0, 28), Ok(()));
    assert_eq!(
        KERNEL_WINDOW.bound_text_write(0x1000, 1),
        Err(WindowError::OutOfTextPage)
    );
}

#[test]
fn a_page_table_leaf_can_be_built_and_read_back() {
    // `mmu` encodes leaves and `split_block` decodes them again; the round trip
    // is what keeps a split equivalent to the block it replaced.
    let leaf = kernel_core::paging::leaf(Level::L3, 0x8000, MemKind::NormalWb, Perms::RX)
        .expect("a page-aligned L3 leaf");
    let (pa, kind, perms) =
        kernel_core::paging::decode_leaf(leaf, Level::L3).expect("what was just encoded");
    assert_eq!(pa, 0x8000);
    assert_eq!(kind, MemKind::NormalWb);
    assert_eq!(perms, Perms::RX);
}

#[test]
fn capability_rights_do_not_imply_each_other() {
    // If these ever collided, every send capability would also be a receive
    // capability and the IPC lookup would stop distinguishing them.
    assert!(!CapRights::SEND.contains(CapRights::RECV));
    assert!(!CapRights::RECV.contains(CapRights::SEND));
    assert!(
        CapRights::SEND
            .union(CapRights::RECV)
            .contains(CapRights::SEND)
    );
}

#[test]
fn the_genet_link_report_is_reachable_from_outside() {
    use kernel_core::genet::{LinkReport, phy};

    assert_eq!(
        LinkReport::from_bmsr(0).to_string(),
        "genet: link=down (bmsr, not a nic)"
    );
    assert_eq!(
        LinkReport::from_bmsr(phy::BMSR_LINK).to_string(),
        "genet: link=up (bmsr, not a nic)"
    );
}

#[test]
fn the_genet_phy_identify_report_is_reachable_from_outside() {
    use kernel_core::genet::PhyIdentify;

    assert_eq!(
        PhyIdentify::from_identify(0x0362, 0x5e60, true).to_string(),
        "genet: phy=0x03625e60 (id, not a nic)"
    );
    assert_eq!(
        PhyIdentify::from_identify(0, 0, true).to_string(),
        "genet: phy unavailable (absent id)"
    );
    assert_eq!(
        PhyIdentify::from_identify(0xffff, 0xffff, true).to_string(),
        "genet: phy unavailable (stuck-high id)"
    );
    assert_eq!(
        PhyIdentify::from_identify(0x0362, 0x5e60, false).to_string(),
        "genet: phy unavailable (mode)"
    );
}

#[test]
fn the_genet_mmio_probe_classifier_is_reachable_from_outside() {
    use kernel_core::genet::{MmioProbe, REGISTER_BYTES, mmio_probe_intent};

    assert_eq!(
        mmio_probe_intent(None, 0xfd58_0000, REGISTER_BYTES),
        Err(MmioProbe::NoBinding)
    );
    assert_eq!(
        mmio_probe_intent(
            Some((0xfd58_0000, REGISTER_BYTES)),
            0xfd58_0000,
            REGISTER_BYTES
        ),
        Ok(())
    );
    assert_eq!(
        MmioProbe::NoBinding.to_string(),
        "genet: probe unavailable (no binding)"
    );
}

#[test]
fn the_genet_fdt_boot_report_is_reachable_from_outside() {
    // What `bootstrap` prints after `discover:` (ADR-0106 report-only):
    // classify the mapped blob, never probe MMIO. An unmapped fixture must
    // stay `NoDtb` even when bytes are supplied.
    use kernel_core::genet_fdt::{Report, Unavailable, boot_report};

    const PI4: &[u8] = include_bytes!("fixtures/bcm2711-rpi-4-b.dtb");
    assert_eq!(
        boot_report(false, Some(PI4)),
        Report::Unavailable(Unavailable::NoDtb)
    );
    let report = boot_report(true, Some(PI4));
    assert!(matches!(report, Report::Binding(_)));
    assert_eq!(
        report.to_string(),
        "genet: binding ok base=0xfd580000 len=0x10000 phy=rgmii-rxid (fdt, not probed)"
    );
}

#[test]
fn the_platform_self_check_surface_is_reachable_from_outside() {
    // What `bootstrap` runs at every boot (ADR-0065): recognise the part,
    // decode the fields the refusals compare, against the values the Pi 4B's
    // Cortex-A72 actually reports. A decode moved behind the crate boundary,
    // or an enum variant the boot line matches on going missing, breaks here
    // rather than in the kernel.
    use kernel_core::cpuid::{self, Part};

    let (midr, mmfr0, pfr0) = (0x410F_D083u64, 0x1124u64, 0x2222u64);
    assert_eq!(cpuid::part(midr), Part::CortexA72);
    assert_eq!(
        (cpuid::variant(midr), cpuid::revision(midr)),
        (0, 3),
        "the r0p3 the boot line prints"
    );
    // The load-bearing conjunction bootstrap refuses without, and the width
    // check's two sides: hardware bits vs the pool's programmed width.
    assert!(cpuid::tgran4_supported(mmfr0));
    assert!(cpuid::el0_aarch64(pfr0) && cpuid::el1_aarch64(pfr0));
    assert!(cpuid::asid_bits(mmfr0).expect("defined encoding") >= kernel_core::asid::ASID_BITS);
    assert_eq!(cpuid::pa_bits(mmfr0), Some(44));
}
