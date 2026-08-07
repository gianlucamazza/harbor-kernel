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
use crate::sched;
use kernel_core::paging::Perms;
use kernel_core::syscall::{self, Syscall};

/// Slot every demo task keeps its console capability in.
///
/// A convention between the spawn sites in `bootstrap` and the programs here,
/// not something the kernel knows: an agent's slots mean whatever its creator
/// put in them, which is the point of naming authority per task (ADR-0017 §2).
pub(super) const CONSOLE_SLOT: u16 = 1;

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
        el0::run(
            sched::current_el0_session(),
            aspace.root_phys(),
            aspace.user_entry_va(),
            aspace.user_sp(),
        )
    });
    match outcome {
        el0::El0Outcome::Svc { imm } => match syscall::decode(imm) {
            Syscall::Ping => println!(uart, "el0: SVC ok  imm=0"),
            Syscall::Exit => println!(uart, "el0: SVC unexpected exit"),

            Syscall::Send => println!(uart, "el0: SVC unexpected send"),
            Syscall::Recv => println!(uart, "el0: SVC unexpected recv"),
            Syscall::TryRecv => println!(uart, "el0: SVC unexpected try-recv"),
            Syscall::WaitIrq => println!(uart, "el0: SVC unexpected wait-irq"),
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
        el0::run(
            sched::current_el0_session(),
            aspace.root_phys(),
            aspace.user_entry_va(),
            aspace.user_sp(),
        )
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

/// Wait until the console server has drained our send-end mailbox (M8 barrier).
fn drain_console_if_held() {
    if let Some(cap) = crate::sched::my_cap(CONSOLE_SLOT as usize) {
        match crate::ipc::yield_until_empty_default(cap) {
            Ok(()) => {}
            Err(e) => crate::kprintln!("console drain wait FAILED {e:?}"),
        }
    }
}

/// M5-P1/P2 + resume/console SEND/IRQ: scheduled task via [`Agent`] shell.
pub(super) fn el0_scheduled_task() {
    let free_before = mm::frames::free_count();
    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-task: create FAILED {e:?}");
            return;
        }
    };

    match agent.run_user_prog(&kernel_core::prog::encode_svc_imm(0)) {
        Ok(out) => agent::report_svc("el0-task", out),
        Err(e) => crate::kprintln!("el0-task: el0 FAILED {e:?}"),
    }

    match agent.run_user_prog(&kernel_core::prog::encode_svc_imm(0x99)) {
        Ok(out) => agent::report_svc("el0-task", out),
        Err(e) => crate::kprintln!("el0-task: refuse path FAILED {e:?}"),
    }

    // Multi-SVC resume: two pings then SYS_EXIT.
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_ping_ping_exit()) {
        Ok(s) if matches!(s.end, agent::SessionEnd::Fault { .. }) => {
            // Reported rather than counted as success: before `SessionEnd`
            // existed, a faulting agent returned `Ok` and the ESR/FAR went
            // nowhere. What to *do* about it is still ADR-0016's open item.
            if let agent::SessionEnd::Fault { esr, far } = s.end {
                crate::kprintln!("el0-task: agent FAULT esr={esr:#x} far={far:#x}");
            }
        }
        Ok(s) if s.pings == 2 && s.sends == 0 => {
            crate::kprintln!("el0-task: resume pings=2");
        }
        Ok(s) => crate::kprintln!(
            "el0-task: resume unexpected pings={} sends={}",
            s.pings,
            s.sends
        ),
        Err(e) => crate::kprintln!("el0-task: resume FAILED {e:?}"),
    }

    // Console endpoint: two bytes via SYS_SEND, drained by the EL1 server.
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_console_hi_exit(CONSOLE_SLOT)) {
        Ok(s) if s.sends == 2 => {
            drain_console_if_held();
            crate::kprintln!("el0-task: console sends=2");
        }
        Ok(s) => crate::kprintln!("el0-task: console unexpected sends={}", s.sends),
        Err(e) => crate::kprintln!("el0-task: console FAILED {e:?}"),
    }

    // EL0 IRQ resume (architectural re-execute): arm the next tick under the
    // EL1 IRQ mask so EL1 does not claim it first; finite spin with EL0 IRQs
    // open; handle + resume re-executes; GPRs survive; SYS_EXIT ends.
    el0::set_entry_irqs_unmasked(sched::current_el0_session());
    match agent.run_user_prog_resuming_prep(&kernel_core::prog::encode_spin_exit(0x800), || {
        timer::accelerate_next_tick(1);
    }) {
        Ok(s) if s.irqs >= 1 => crate::kprintln!("el0-task: irq resume irqs={}", s.irqs),
        Ok(s) => crate::kprintln!("el0-task: irq resume unexpected irqs={}", s.irqs),
        Err(e) => crate::kprintln!("el0-task: irq resume FAILED {e:?}"),
    }
    el0::set_entry_irqs_masked(sched::current_el0_session());

    agent.destroy();
    // Pool free-count equality is not a per-task invariant under concurrent
    // agents (other tasks allocate and free frames while this one runs). The
    // agent AS is destroyed above; report the pool for the concurrent smoke.
    let free_after = mm::frames::free_count();
    crate::kprintln!("el0-task: ok  pool={free_after} (was {free_before})");
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
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_pl011_rx_poll_exit(CONSOLE_SLOT))
    {
        Ok(s) if s.sends == 0 => crate::kprintln!("pl011-agent: rx poll empty"),
        Ok(s) => crate::kprintln!("pl011-agent: rx poll unexpected sends={}", s.sends),
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
        match agent
            .run_user_prog_resuming(&kernel_core::prog::encode_pl011_rx_poll_exit(CONSOLE_SLOT))
        {
            Ok(s) if s.sends == 1 => got = got.saturating_add(1),
            Ok(s) if s.sends == 0 => {
                crate::kprintln!("pl011-agent: rx own short sends=0 after {got}");
                break;
            }
            Ok(s) => {
                crate::kprintln!("pl011-agent: rx own unexpected sends={}", s.sends);
                break;
            }
            Err(e) => {
                crate::kprintln!("pl011-agent: rx own FAILED {e:?}");
                break;
            }
        }
    }

    if got == OWN_BYTES.len() as u32 {
        drain_console_if_held();
        crate::kprintln!("pl011-agent: rx own bytes={got}");
    } else {
        crate::kprintln!("pl011-agent: rx own incomplete got={got}");
    }

    console::resume_rx(rx_base);
    crate::kprintln!("pl011-agent: rx own end");
    crate::sched::yield_now();

    agent.destroy();
    // Same as el0-task: global free_count is not exclusive under concurrency.
    let free_after = mm::frames::free_count();
    crate::kprintln!("pl011-agent: killed ok  pool={free_after} (was {free_before})");
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

/// M7 slice 2: an EL0 agent that sends through the one slot it holds, and then
/// one that reaches for a slot it does not.
///
/// Runs **second** of the pair, and that is the assertion. The receiver is
/// spawned first and reaches its `SYS_RECV` on an empty mailbox, so the payload
/// crossing at all means the kernel parked it and this send woke it
/// (ADR-0022 §1). Before the park existed the order was the other way round and
/// the receiver opened with two `yield_now()` — ordering by construction, which
/// is the arrangement that keeps a property from being tested.
pub(super) fn el0_ipc_sender() {
    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-ipc: sender create FAILED {e:?}");
            return;
        }
    };

    // Slot 0 is the send capability this task was spawned holding. `42` is the
    // payload and also `*`, which is what the receiving agent will print.
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_send_exit(0, 7, 42)) {
        Ok(s) if s.sends == 1 && s.authority_refusals == 0 => {
            crate::kprintln!("el0-ipc: sent slot=0 tag=7 a=42")
        }
        Ok(s) => crate::kprintln!(
            "el0-ipc: send unexpected sends={} refusals={}",
            s.sends,
            s.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-ipc: send FAILED {e:?}"),
    }

    // Denied the console, deliberately. This task was spawned holding only a
    // send capability, so `CONSOLE_SLOT` is empty in *its* table — and the byte
    // it tries to print never reaches the UART. ADR-0017 §3 requires exactly
    // this on the good path: a capability nobody is ever seen to lack is a
    // protection nobody has seen fire.
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_console_once_exit(
        CONSOLE_SLOT,
        b'X',
    )) {
        Ok(s) if s.sends == 0 && s.authority_refusals == 1 => {
            crate::kprintln!("el0-ipc: console denied, printed nothing")
        }
        Ok(s) => crate::kprintln!(
            "el0-ipc: console denial unexpected sends={} refusals={}",
            s.sends,
            s.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-ipc: console denial FAILED {e:?}"),
    }

    // The same agent, now naming slot 1 for a *send*. This task holds exactly
    // one capability, so slot 1 is empty and there is nothing there to name —
    // the refusal is structural, not a check that happened to be written.
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_send_bare_exit(1)) {
        Ok(s) if s.authority_refusals == 1 && s.sends == 0 => crate::kprintln!(
            "el0-ipc: refused slot=1 authority={}",
            crate::ipc::refused_count()
        ),
        Ok(s) => crate::kprintln!(
            "el0-ipc: refuse unexpected sends={} refusals={}",
            s.sends,
            s.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-ipc: refuse FAILED {e:?}"),
    }

    // ADR-0018: the agent faults, the kernel ends its session, and *this task*
    // — the creator, which allocated the address space and granted the slots —
    // decides what happens next. It decides to carry on: the fault is reported,
    // the agent is not restarted, and the peer that is still waiting for the
    // message this task already sent is unaffected.
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_fault_exit()) {
        Ok(s) => match s.end {
            agent::SessionEnd::Fault { esr, far } => crate::kprintln!(
                "el0-ipc: agent faulted esr={esr:#x} far={far:#x} faults={}",
                agent::fault_count()
            ),
            other => crate::kprintln!("el0-ipc: fault unexpected end {other:?}"),
        },
        Err(e) => crate::kprintln!("el0-ipc: fault path FAILED {e:?}"),
    }
    crate::kprintln!("el0-ipc: creator alive after fault");

    agent.destroy();
}

/// M7 slice 2: an EL0 agent that receives through the one slot it holds and
/// prints the payload itself.
///
/// The two tasks hold *different* capability tables, which is the point: slot 0
/// here and slot 0 there name different objects, and neither agent can name the
/// other's. A slot index is meaningless outside the task that owns the table.
pub(super) fn el0_ipc_receiver() {
    // No `yield_now` here, deliberately. This task is spawned before the sender
    // and runs first, so its `SYS_RECV` finds an empty mailbox and parks. If the
    // recv stopped parking, the agent would read `Empty` and print the failure
    // branch below — the removal of those yields is the test, not its setting.
    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-ipc: receiver create FAILED {e:?}");
            return;
        }
    };

    // First, the non-blocking half on the very same slot (ADR-0022 §4). The
    // mailbox is empty — the sender has not run — so this is the one program in
    // the tree that still reaches `Status::Empty`, and it reaches it on the good
    // path where a branch nobody sees taken would otherwise rot. It also says
    // something the blocking recv below cannot: that the mailbox really was
    // empty when this agent got here, so the park that follows is a park.
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_try_recv_exit(0)) {
        Ok(s) if s.recv_empties == 1 && s.recvs == 0 && s.authority_refusals == 0 => {
            crate::kprintln!("el0-ipc: try-recv empty without waiting empties=1")
        }
        Ok(s) => crate::kprintln!(
            "el0-ipc: try-recv unexpected empties={} recvs={} refusals={}",
            s.recv_empties,
            s.recvs,
            s.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-ipc: try-recv FAILED {e:?}"),
    }

    // Then the waiting half, on the same empty mailbox. One console SEND, and the byte
    // it prints is the payload the other agent sent — after this task parked and
    // that send woke it.
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_recv_console_exit(
        0,
        CONSOLE_SLOT,
    )) {
        Ok(s) if s.recvs == 1 && s.sends == 1 => {
            drain_console_if_held();
            crate::kprintln!("el0-ipc: got payload via EL0 recvs=1")
        }
        Ok(s) => crate::kprintln!(
            "el0-ipc: recv unexpected recvs={} sends={} refusals={}",
            s.recvs,
            s.sends,
            s.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-ipc: recv FAILED {e:?}"),
    }

    agent.destroy();
}

/// ADR-0025: published so the reaper can cancel this task after it parks.
pub(super) static ORPHAN_TASK: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// ADR-0025: parks on a mailbox nobody will send to; expects supervisor cancel.
pub(super) fn orphan_receiver() {
    let Some(cap) = crate::sched::my_cap(0) else {
        crate::kprintln!("ipc: orphan has no cap");
        return;
    };
    match crate::ipc::recv(cap) {
        Err(crate::ipc::RecvError::Cancelled) => {
            crate::kprintln!("ipc: reaped cancelled")
        }
        Ok(msg) => crate::kprintln!("ipc: reaped unexpected tag={} a={}", msg.tag, msg.a),
        Err(e) => crate::kprintln!("ipc: reaped FAILED {e:?}"),
    }
}

/// ADR-0025: after the orphan parks, cancel its wait (creator/supervisor role).
pub(super) fn orphan_reaper() {
    // Two yields: orphan must run and block first.
    crate::sched::yield_now();
    crate::sched::yield_now();
    let raw = ORPHAN_TASK.load(core::sync::atomic::Ordering::Relaxed);
    if raw == u32::MAX {
        crate::kprintln!("ipc: reaper has no orphan id");
        return;
    }
    let id = kernel_core::runqueue::TaskId(raw);
    if crate::ipc::cancel_blocked(id) {
        crate::kprintln!(
            "ipc: cancel issued cancel_events={}",
            crate::sched::cancel_events()
        );
    } else {
        crate::kprintln!("ipc: cancel FAILED (not blocked?)");
    }
    crate::sched::yield_now();
}

/// ADR-0031 / K2: parks on an ephemeral channel; sole SEND holder will exit.
pub(super) fn auto_reap_receiver() {
    let Some(cap) = crate::sched::my_cap(0) else {
        crate::kprintln!("ipc: auto-reap recv has no cap");
        return;
    };
    match crate::ipc::recv(cap) {
        Err(crate::ipc::RecvError::Cancelled) => {
            crate::kprintln!("ipc: auto-reaped cancelled")
        }
        Ok(msg) => crate::kprintln!("ipc: auto-reap unexpected tag={} a={}", msg.tag, msg.a),
        Err(e) => crate::kprintln!("ipc: auto-reap FAILED {e:?}"),
    }
}

/// ADR-0031: holds SEND, yields so the peer parks, then exits (drops the hold).
pub(super) fn auto_reap_sender() {
    crate::sched::yield_now();
    crate::sched::yield_now();
    // Exit without sending — last SEND hold drop auto-cancels the waiter.
}

/// K1 / ADR-0028 + ADR-0030: EL1 timer wait, then EL0 `SYS_WAIT_IRQ` on the
/// same cookie (sequential so the one-waiter table is free).
///
/// Cookie `1` is what `bsp::rpi4::irq` registers for the arch timer. Slot 0 of
/// this task holds the timer IRQ notification (minted at bootstrap).
pub(super) fn irq_wait_task() {
    crate::kprintln!("irq-wait: arm cookie=1");
    // Timer runs at TIMER_HZ (10 Hz). One period is enough evidence.
    match crate::sched::wait_for_irq(1) {
        Ok(()) => crate::kprintln!(
            "irq-wait: woke drops={} idle_signals={}",
            crate::sched::wake_drops(),
            crate::irq::wait::signal_idle()
        ),
        Err(e) => crate::kprintln!("irq-wait: arm FAILED {e:?}"),
    }

    // ADR-0030: EL0 parks via a granted notification in slot 0 (not a raw cookie).
    let mut agent = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-irq: create FAILED {e:?}");
            return;
        }
    };
    crate::kprintln!("el0-irq: arm slot=0");
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_wait_irq_exit(0)) {
        Ok(stats) if stats.wait_irqs >= 1 && stats.authority_refusals == 0 => {
            crate::kprintln!("el0-irq: woke wait_irqs={}", stats.wait_irqs);
        }
        Ok(stats) => crate::kprintln!(
            "el0-irq: unexpected end={:?} wait_irqs={} refusals={}",
            stats.end,
            stats.wait_irqs,
            stats.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-irq: run FAILED {e:?}"),
    }
    agent.destroy();
}

/// ADR-0030: `SYS_WAIT_IRQ` with an empty slot must be Authority (seen on the good path).
pub(super) fn el0_irq_refuse_task() {
    let mut agent = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-irq-refuse: create FAILED {e:?}");
            return;
        }
    };
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_wait_irq_exit(0)) {
        Ok(stats) if stats.authority_refusals >= 1 => {
            crate::kprintln!("el0-irq: refused refusals={}", stats.authority_refusals);
        }
        Ok(stats) => crate::kprintln!(
            "el0-irq-refuse: unexpected end={:?} refusals={}",
            stats.end,
            stats.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-irq-refuse: run FAILED {e:?}"),
    }
    agent.destroy();
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
    // ADR-0024: parks are countable. Yield so the console server can re-park
    // on its empty mailbox before we sample; block_events still counts every
    // successful park even if nobody is blocked at the sample.
    crate::sched::yield_now();
    crate::kprintln!(
        "sched: blocked={} block_events={}",
        crate::sched::blocked_count(),
        crate::sched::block_events()
    );
}
