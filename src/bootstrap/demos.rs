//! Demonstrations and hardware smoke bodies spawned by [`super::run`].
//!
//! Split out of `bootstrap::mod`, whose doc-comment has always said it holds
//! "the ordered sequence … and nothing else" while carrying eight demo and
//! smoke functions — 367 of its 773 lines. The claim is now true: `mod.rs` is
//! the sequence, and what the machine then *does* to prove each milestone is
//! here.
//!
//! Everything in this file exists to produce a line the QEMU boot check asserts
//! on. That is why it is in a production image at all, and it is the reason to
//! be uncomfortable about it: `selftest.rs` is behind `--features bringup` for
//! exactly this reason, and these bodies are not. Moving them behind a feature
//! means the boot oracle no longer describes the image that ships, which is a
//! trade this audit records rather than settles.

use crate::agent::{self, Agent};
use crate::arch::{cpu, el0, timer};
use crate::console;
use crate::drivers::pl011::Pl011;
use crate::mm;
use crate::println;
use kernel_core::paging::Perms;
use kernel_core::syscall::{self, Syscall};

/// Bit pattern of a valid send cap the forger does **not** hold (M4 refuse).
pub(super) static IPC_FORGE_RAW: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// M5: prepare + SVC/fault probes + dual-AS teardown (bootstrap, before sched).
pub(super) fn m5_aspace_and_el0_smoke(uart: &mut Pl011) {
    let free_before = mm::frames::free_count();
    let mut aspace = match mm::AddressSpace::create() {
        Ok(a) => a,
        Err(error) => {
            println!(uart, "aspace: create FAILED {error:?}");
            return;
        }
    };
    let held_empty = aspace.frame_count();
    if let Err(error) = aspace.prepare_for_el0() {
        println!(uart, "aspace: prepare FAILED {error:?}");
        aspace.destroy();
        return;
    }
    let held = aspace.frame_count();
    println!(
        uart,
        "aspace: prepare ok  held={held} (empty={held_empty})  root={:#x}",
        aspace.root_phys()
    );

    // A64: SVC #0 ; B .
    let svc_prog: [u8; 8] = [
        0x01, 0x00, 0x00, 0xD4, // svc #0
        0x00, 0x00, 0x00, 0x14, // b .
    ];
    // A64: MOVZ X0, #8, LSL#16  (0x80000) ; STR XZR, [X0] ; SVC #0
    let fault_prog: [u8; 12] = [
        0x00, 0x01, 0xA0, 0xD2, // movz x0, #0x8, lsl #16
        0x1F, 0x00, 0x00, 0xF9, // str xzr, [x0]
        0x01, 0x00, 0x00, 0xD4, // svc #0
    ];

    if aspace.poke_user(0, &svc_prog).is_err() {
        println!(uart, "aspace: poke FAILED");
        aspace.destroy();
        return;
    }
    // SAFETY: the AS was prepared above, the entry runs inside `without_irqs`
    // so EL1 IRQs are masked as `el0::run` requires, and this is the sole
    // session — bootstrap is single-threaded here, before the scheduler starts
    // (ADR-0016).
    // SAFETY: as the probe above — the AS is prepared, the entry runs inside `without_irqs`
    // so EL1 IRQs are masked as `el0::run` requires, and this is the second of two
    // one-shot sessions run one after the other, never overlapping (ADR-0016).
    let outcome = cpu::without_irqs(|| unsafe {
        el0::run(aspace.root_phys(), aspace.user_entry_va(), aspace.user_sp())
    });
    match outcome {
        el0::El0Outcome::Svc { imm } => match syscall::decode(imm) {
            Syscall::Ping => println!(uart, "el0: SVC ok  imm=0"),
            Syscall::Exit => println!(uart, "el0: SVC unexpected exit"),
            Syscall::Putc => println!(uart, "el0: SVC unexpected putc"),
            Syscall::Unknown { imm } => println!(uart, "el0: SVC unexpected imm={imm}"),
        },
        other => println!(uart, "el0: SVC unexpected {other:?}"),
    }

    if aspace.poke_user(0, &fault_prog).is_err() {
        println!(uart, "aspace: poke2 FAILED");
        aspace.destroy();
        return;
    }
    // SAFETY: as the two sessions above. This one is expected to fault — the
    // program stores to a kernel address — which is still a terminating outcome
    // for a one-shot session, not a resumable one.
    let outcome = cpu::without_irqs(|| unsafe {
        el0::run(aspace.root_phys(), aspace.user_entry_va(), aspace.user_sp())
    });
    match outcome {
        el0::El0Outcome::DataAbort { esr, far } => {
            println!(uart, "el0: FAULT ok  ESR={esr:#x} FAR={far:#x}");
        }
        other => println!(uart, "el0: FAULT unexpected {other:?}"),
    }

    aspace.destroy();
    let free_after = mm::frames::free_count();
    if free_after == free_before {
        println!(uart, "aspace: create/destroy ok  pool={free_after}");
    } else {
        println!(uart, "aspace: LEAK  free {free_before}->{free_after}");
    }

    // M5-P3: two AS live, then both destroyed — pool must restore.
    let free_dual = mm::frames::free_count();
    let Ok(mut a) = mm::AddressSpace::create() else {
        println!(uart, "aspace: dual create-a FAILED");
        return;
    };
    let Ok(mut b) = mm::AddressSpace::create() else {
        println!(uart, "aspace: dual create-b FAILED");
        a.destroy();
        return;
    };
    if a.prepare_for_el0().is_err() || b.prepare_for_el0().is_err() {
        println!(uart, "aspace: dual prepare FAILED");
        a.destroy();
        b.destroy();
        return;
    }
    a.destroy();
    b.destroy();
    let free_end = mm::frames::free_count();
    if free_end == free_dual {
        println!(uart, "aspace: dual create/destroy ok  pool={free_end}");
    } else {
        println!(uart, "aspace: dual LEAK  free {free_dual}->{free_end}");
    }
}

/// M5-P1/P2 + resume/putc/IRQ: scheduled task via [`Agent`] shell.
pub(super) fn el0_scheduled_task() {
    let free_before = mm::frames::free_count();
    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-task: create FAILED {e:?}");
            return;
        }
    };

    match agent.run_user_prog(&agent::encode_svc_imm(0)) {
        Ok(out) => agent::report_svc("el0-task", out),
        Err(e) => crate::kprintln!("el0-task: el0 FAILED {e:?}"),
    }

    match agent.run_user_prog(&agent::encode_svc_imm(0x99)) {
        Ok(out) => agent::report_svc("el0-task", out),
        Err(e) => crate::kprintln!("el0-task: refuse path FAILED {e:?}"),
    }

    // Multi-SVC resume: two pings then SYS_EXIT.
    match agent.run_user_prog_resuming(&agent::encode_ping_ping_exit()) {
        Ok(s) if matches!(s.end, agent::SessionEnd::Fault { .. }) => {
            // Reported rather than counted as success: before `SessionEnd`
            // existed, a faulting agent returned `Ok` and the ESR/FAR went
            // nowhere. What to *do* about it is still ADR-0016's open item.
            if let agent::SessionEnd::Fault { esr, far } = s.end {
                crate::kprintln!("el0-task: agent FAULT esr={esr:#x} far={far:#x}");
            }
        }
        Ok(s) if s.pings == 2 && s.putcs == 0 => {
            crate::kprintln!("el0-task: resume pings=2");
        }
        Ok(s) => crate::kprintln!(
            "el0-task: resume unexpected pings={} putcs={}",
            s.pings,
            s.putcs
        ),
        Err(e) => crate::kprintln!("el0-task: resume FAILED {e:?}"),
    }

    // SYS_PUTC: two bytes via kernel TX, then exit.
    match agent.run_user_prog_resuming(&agent::encode_putc_hi_exit()) {
        Ok(s) if s.putcs == 2 => crate::kprintln!("el0-task: putc bytes=2"),
        Ok(s) => crate::kprintln!("el0-task: putc unexpected putcs={}", s.putcs),
        Err(e) => crate::kprintln!("el0-task: putc FAILED {e:?}"),
    }

    // EL0 IRQ resume (architectural re-execute): arm the next tick under the
    // EL1 IRQ mask so EL1 does not claim it first; finite spin with EL0 IRQs
    // open; handle + resume re-executes; GPRs survive; SYS_EXIT ends.
    el0::set_entry_irqs_unmasked();
    match agent.run_user_prog_resuming_prep(&agent::encode_spin_exit(0x800), || {
        timer::accelerate_next_tick(1);
    }) {
        Ok(s) if s.irqs >= 1 => crate::kprintln!("el0-task: irq resume irqs={}", s.irqs),
        Ok(s) => crate::kprintln!("el0-task: irq resume unexpected irqs={}", s.irqs),
        Err(e) => crate::kprintln!("el0-task: irq resume FAILED {e:?}"),
    }
    el0::set_entry_irqs_masked();

    agent.destroy();
    let free_after = mm::frames::free_count();
    if free_after == free_before {
        crate::kprintln!("el0-task: ok");
    } else {
        crate::kprintln!("el0-task: LEAK {free_before}->{free_after}");
    }
}

/// M6: PL011 page agent (ADR-0013) + RX ownership (poll) with real bytes.
///
/// Ownership window: kernel drain suspended, PL011 RX IRQs masked, agent maps
/// the page and polls `DR`. Self-test uses PL011 loopback (no host typing) so
/// QEMU and silicon share the same oracle. Yields so idle can still tick.
/// Destroy = kill (unmap); drain restored before return.
pub(super) fn pl011_agent_task() {
    use crate::bsp::board::memmap::{FRAME_SIZE, UART0_BASE, UART0_REG_BYTES, USER_PL011_VA};
    use kernel_core::a64;

    if UART0_REG_BYTES != FRAME_SIZE {
        crate::kprintln!("pl011-agent: UART0_REG_BYTES must be one page");
        return;
    }

    let free_before = mm::frames::free_count();
    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("pl011-agent: create FAILED {e:?}");
            return;
        }
    };
    if let Err(e) =
        agent
            .aspace_mut()
            .map_device_page(USER_PL011_VA, UART0_BASE as u64, Perms::USER_RW)
    {
        crate::kprintln!("pl011-agent: map FAILED {e:?}");
        agent.destroy();
        return;
    }

    // FR load + ping (map liveness).
    let mut fr_prog = [0u8; 12];
    let w0 = a64::le_bytes(a64::movz_x_lsl16(0, 0x5000));
    let w1 = a64::le_bytes(a64::ldr_w_imm(1, 0, 0x18));
    let w2 = a64::le_bytes(a64::svc(0));
    fr_prog[0..4].copy_from_slice(&w0);
    fr_prog[4..8].copy_from_slice(&w1);
    fr_prog[8..12].copy_from_slice(&w2);

    match agent.run_user_prog(&fr_prog) {
        Ok(el0::El0Outcome::Svc { imm }) if matches!(syscall::decode(imm), Syscall::Ping) => {
            crate::kprintln!("pl011-agent: FR read + svc ok");
        }
        Ok(other) => crate::kprintln!("pl011-agent: unexpected {other:?}"),
        Err(e) => crate::kprintln!("pl011-agent: el0 FAILED {e:?}"),
    }

    // --- RX ownership (poll) with real bytes via PL011 loopback ---
    let rx_base = console::suspend_rx();
    if rx_base == 0 {
        // The kernel still owns the drain, so a poll here would race it and
        // report bytes nobody handed over. Say so instead of measuring noise.
        crate::kprintln!("pl011-agent: rx own SKIPPED (drain not suspended)");
        agent.destroy();
        return;
    }
    crate::kprintln!("pl011-agent: rx own begin");
    crate::sched::yield_now();

    // Empty path first (honest): no invented data.
    match agent.run_user_prog_resuming(&agent::encode_pl011_rx_poll_exit()) {
        Ok(s) if s.putcs == 0 => crate::kprintln!("pl011-agent: rx poll empty"),
        Ok(s) => crate::kprintln!("pl011-agent: rx poll unexpected putcs={}", s.putcs),
        Err(e) => crate::kprintln!("pl011-agent: rx poll FAILED {e:?}"),
    }
    crate::sched::yield_now();

    // Inject two bytes through hardware loopback (kernel TX → internal RX).
    const OWN_BYTES: &[u8] = b"RX";
    let injected = console::with_tx(|uart| {
        uart.set_loopback(true);
        uart.receiver().discard_and_ack();
        let ok = uart.write_bytes(OWN_BYTES);
        uart.set_loopback(false);
        ok
    });
    if injected != Some(true) {
        crate::kprintln!("pl011-agent: rx own inject FAILED");
        console::resume_rx(rx_base);
        agent.destroy();
        return;
    }

    let mut got = 0u32;
    for _ in 0..OWN_BYTES.len() {
        crate::sched::yield_now();
        match agent.run_user_prog_resuming(&agent::encode_pl011_rx_poll_exit()) {
            Ok(s) if s.putcs == 1 => got = got.saturating_add(1),
            Ok(s) if s.putcs == 0 => {
                crate::kprintln!("pl011-agent: rx own short putcs=0 after {got}");
                break;
            }
            Ok(s) => {
                crate::kprintln!("pl011-agent: rx own unexpected putcs={}", s.putcs);
                break;
            }
            Err(e) => {
                crate::kprintln!("pl011-agent: rx own FAILED {e:?}");
                break;
            }
        }
    }

    if got == OWN_BYTES.len() as u32 {
        crate::kprintln!("pl011-agent: rx own bytes={got}");
    } else {
        crate::kprintln!("pl011-agent: rx own incomplete got={got}");
    }

    console::resume_rx(rx_base);
    crate::kprintln!("pl011-agent: rx own end");
    crate::sched::yield_now();

    agent.destroy();
    let free_after = mm::frames::free_count();
    if free_after == free_before {
        crate::kprintln!("pl011-agent: killed ok  pool={free_after}");
    } else {
        crate::kprintln!("pl011-agent: LEAK {free_before}->{free_after}");
    }
}

/// M3 demo: yield so the peer's lines interleave on the console.
pub(super) fn demo_task_a() {
    for i in 0..4 {
        crate::kprintln!("task-a {i}");
        crate::sched::yield_now();
    }
}

pub(super) fn demo_task_b() {
    for i in 0..4 {
        crate::kprintln!("task-b {i}");
        crate::sched::yield_now();
    }
}

/// M4: holds recv cap only; blocks until sender posts.
pub(super) fn ipc_receiver() {
    let Some(cap) = crate::sched::my_cap(0) else {
        crate::kprintln!("ipc: receiver has no cap");
        return;
    };
    match crate::ipc::recv(cap) {
        Ok(msg) => crate::kprintln!("ipc: got tag={} a={}", msg.tag, msg.a),
        Err(e) => crate::kprintln!("ipc: recv FAILED {e:?}"),
    }
}

/// M4: holds send cap; delivers one message across the mailbox.
pub(super) fn ipc_sender() {
    // Let the receiver block first so the wait/wake path is exercised.
    crate::sched::yield_now();
    let Some(cap) = crate::sched::my_cap(0) else {
        crate::kprintln!("ipc: sender has no cap");
        return;
    };
    let msg = crate::ipc::Message {
        tag: 1,
        a: 42,
        b: 0,
    };
    match crate::ipc::send(cap, msg) {
        Ok(()) => crate::kprintln!("ipc: sent tag=1 a=42"),
        Err(e) => crate::kprintln!("ipc: send FAILED {e:?}"),
    }
}

/// M4: knows the send-cap bit pattern but does not hold it — must refuse.
pub(super) fn ipc_forger() {
    crate::sched::yield_now();
    crate::sched::yield_now();
    let raw = IPC_FORGE_RAW.load(core::sync::atomic::Ordering::Relaxed);
    let stolen = kernel_core::cap::CapId::from_raw(raw);
    let msg = crate::ipc::Message {
        tag: 99,
        a: 0,
        b: 0,
    };
    match crate::ipc::send(stolen, msg) {
        Ok(()) => crate::kprintln!("ipc: FORGE OK — capability check failed"),
        Err(_) => crate::kprintln!(
            "ipc: refuse count={} full={} state={}",
            crate::ipc::refused_count(),
            crate::ipc::refused_full_count(),
            crate::ipc::refused_state_count()
        ),
    }
}
