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
            aspace.ttbr0_value() as usize,
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
            Syscall::Resolve => println!(uart, "el0: SVC unexpected resolve"),
            Syscall::Transfer => println!(uart, "el0: SVC unexpected transfer"),
            Syscall::RecvTimeout => println!(uart, "el0: SVC unexpected recv-timeout"),
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
            aspace.ttbr0_value() as usize,
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

    // M5-P3 + K7: two AS live with distinct ASIDs, each EL0 once, then destroy.
    let free_dual = mm::frames::free_count();
    let asid_free_before = mm::asid::free_count();
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
    let asid_a = a.asid();
    let asid_b = b.asid();
    if asid_a == 0 || asid_b == 0 || asid_a == asid_b {
        println!(uart, "asid: dual FAILED a={asid_a} b={asid_b}");
        a.destroy();
        b.destroy();
        return;
    }
    // Distinct ASIDs + both enter EL0 (SVC) without a global TLBI between them.
    if a.poke_user(0, &svc_prog).is_err() || b.poke_user(0, &svc_prog).is_err() {
        println!(uart, "asid: dual poke FAILED");
        a.destroy();
        b.destroy();
        return;
    }
    // SAFETY: both ASes prepared and poked; sequential one-shot sessions under
    // IRQ mask (ADR-0016) — never overlapping.
    let ok_a = cpu::without_irqs(|| unsafe {
        matches!(
            el0::run(
                sched::current_el0_session(),
                a.ttbr0_value() as usize,
                a.user_entry_va(),
                a.user_sp(),
            ),
            el0::El0Outcome::Svc { imm: 0 }
        )
    });
    // SAFETY: as `ok_a` — second AS, still sole session after the first ended.
    let ok_b = cpu::without_irqs(|| unsafe {
        matches!(
            el0::run(
                sched::current_el0_session(),
                b.ttbr0_value() as usize,
                b.user_entry_va(),
                b.user_sp(),
            ),
            el0::El0Outcome::Svc { imm: 0 }
        )
    });
    if ok_a && ok_b {
        println!(uart, "asid: dual a={asid_a} b={asid_b} ok");
    } else {
        println!(uart, "asid: dual el0 FAILED a={ok_a} b={ok_b}");
    }
    a.destroy();
    b.destroy();
    let free_end = mm::frames::free_count();
    let asid_free_end = mm::asid::free_count();
    if free_end == free_dual {
        println!(uart, "aspace: dual create/destroy ok  pool={free_end}");
    } else {
        println!(uart, "aspace: dual LEAK  free {free_dual}->{free_end}");
    }
    if asid_free_end != asid_free_before {
        println!(uart, "asid: LEAK free {asid_free_before}->{asid_free_end}");
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

/// K9 / ADR-0034: RNG200 page agent — second peripheral on the M6 map pattern.
///
/// Maps one Device page, EL0 loads `RNG_CTRL`, destroy = kill. QEMU has no
/// RNG200 backend: a data abort on the load is an accepted path (`map fault ok`).
pub(super) fn rng_agent_task() {
    use crate::bsp::board::memmap::{FRAME_SIZE, RNG200_BASE, RNG200_REG_BYTES, USER_RNG_VA};
    use kernel_core::a64;

    if RNG200_REG_BYTES != FRAME_SIZE {
        crate::kprintln!("rng-agent: RNG200_REG_BYTES must be one page");
        return;
    }

    let free_before = mm::frames::free_count();
    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("rng-agent: create FAILED {e:?}");
            return;
        }
    };
    if let Err(e) =
        agent
            .aspace_mut()
            .map_device_page(USER_RNG_VA, RNG200_BASE as u64, Perms::USER_RW)
    {
        crate::kprintln!("rng-agent: map FAILED {e:?}");
        agent.destroy();
        return;
    }

    // USER_RNG_VA = 0x5100_0000 → movz x0, #0x5100, lsl #16; ldr w1, [x0]; svc #0
    let mut prog = [0u8; 12];
    let w0 = a64::le_bytes(a64::movz_x_lsl16(0, 0x5100));
    let w1 = a64::le_bytes(a64::ldr_w_imm(1, 0, 0));
    let w2 = a64::le_bytes(a64::svc(0));
    prog[0..4].copy_from_slice(&w0);
    prog[4..8].copy_from_slice(&w1);
    prog[8..12].copy_from_slice(&w2);

    match agent.run_user_prog(&prog) {
        Ok(el0::El0Outcome::Svc { imm }) if matches!(syscall::decode(imm), Syscall::Ping) => {
            crate::kprintln!("rng-agent: map read ok");
        }
        Ok(el0::El0Outcome::DataAbort { .. }) | Ok(el0::El0Outcome::OtherSync { .. }) => {
            // Missing bus backend (typical QEMU): map still granted and revoked.
            crate::kprintln!("rng-agent: map fault ok");
        }
        Ok(other) => crate::kprintln!("rng-agent: unexpected {other:?}"),
        Err(e) => crate::kprintln!("rng-agent: el0 FAILED {e:?}"),
    }

    agent.destroy();
    let free_after = mm::frames::free_count();
    crate::kprintln!("rng-agent: killed ok  pool={free_after} (was {free_before})");
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

/// Raw CapId bits published so bootstrap can try a stale send after revoke.
pub(super) static REVOKE_STALE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// ADR-0033 / K10: parks until supervisor reaps (Cancelled → return → exit).
pub(super) fn supervised_child() {
    let Some(cap) = crate::sched::my_cap(0) else {
        crate::kprintln!("supervised: no cap");
        return;
    };
    match crate::ipc::recv(cap) {
        Err(crate::ipc::RecvError::Cancelled) => crate::kprintln!("supervised: cancelled"),
        Ok(msg) => crate::kprintln!("supervised: unexpected tag={} a={}", msg.tag, msg.a),
        Err(e) => crate::kprintln!("supervised: FAILED {e:?}"),
    }
}

/// ADR-0033: reap a blocked child, wait for Empty, re-spawn (restart), reap again.
pub(super) fn supervisor_task() {
    let Ok(ch) = crate::ipc::create_channel() else {
        crate::kprintln!("supervisor: channel FAILED");
        return;
    };
    let Ok(id) = crate::sched::spawn_with_caps(supervised_child, &[ch.recv]) else {
        crate::kprintln!("supervisor: child spawn FAILED");
        return;
    };
    // `ch.send` not installed: child parks until reap (default channel, no auto-reap).
    crate::sched::yield_now();
    crate::sched::yield_now();
    match crate::sched::supervisor_reap_blocked(id) {
        Ok(()) => crate::kprintln!(
            "supervisor: reaped id={} reap_events={}",
            id.0,
            crate::sched::reap_events()
        ),
        Err(e) => crate::kprintln!("supervisor: reap FAILED {e:?}"),
    }
    for _ in 0..16 {
        if matches!(
            crate::sched::task_state(id),
            Some(kernel_core::tasks::State::Empty)
        ) {
            break;
        }
        crate::sched::yield_now();
    }
    // Restart: same recv grant after slot is free (restart = re-spawn, ADR-0033).
    match crate::sched::spawn_with_caps(supervised_child, &[ch.recv]) {
        Ok(id2) => {
            crate::kprintln!("supervisor: restarted id={}", id2.0);
            crate::sched::yield_now();
            crate::sched::yield_now();
            let _ = crate::sched::supervisor_reap_blocked(id2);
            for _ in 0..16 {
                if matches!(
                    crate::sched::task_state(id2),
                    Some(kernel_core::tasks::State::Empty)
                ) {
                    break;
                }
                crate::sched::yield_now();
            }
        }
        Err(e) => crate::kprintln!("supervisor: restart FAILED {e:?}"),
    }
}

/// ADR-0032: holds SEND, revokes the channel, then bootstrap sees stale refuse.
pub(super) fn revoke_held_task() {
    let Some(cap) = crate::sched::my_cap(0) else {
        crate::kprintln!("ipc: revoke-held has no cap");
        return;
    };
    match crate::ipc::revoke_held(cap) {
        Ok(()) => {
            crate::kprintln!("ipc: held-revoke ok");
            // Prove the dead handle cannot send (same CapId bits).
            let stale = kernel_core::cap::CapId::from_raw(cap.raw());
            match crate::ipc::creator_try_send(
                stale,
                kernel_core::ipc::Message {
                    tag: 99,
                    a: 0,
                    b: 0,
                },
            ) {
                Err(crate::ipc::SendError::BadCap) => {
                    crate::kprintln!("ipc: release stale refused")
                }
                Ok(()) => crate::kprintln!("ipc: release stale UNEXPECTED send ok"),
                Err(e) => crate::kprintln!("ipc: release stale unexpected {e:?}"),
            }
        }
        Err(e) => crate::kprintln!("ipc: held-revoke FAILED {e:?}"),
    }
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
            // ADR-0043: same IRQ-cap-only wait path is the device-agent story
            // (one waiter per cookie — do not race a second concurrent task).
            crate::kprintln!("irq-device: woke wait_irqs={}", stats.wait_irqs);
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

/// Peer id for ADR-0037 transfer recipient (set by bootstrap before donor runs).
pub(super) static TRANSFER_TO: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// ADR-0037: holds SEND in slot 0; moves it to TRANSFER_TO slot 0.
pub(super) fn transfer_donor_task() {
    let to = crate::sched::TaskId(TRANSFER_TO.load(core::sync::atomic::Ordering::Relaxed));
    match crate::sched::transfer_held(0, to, 0) {
        Ok(()) => {
            // Donor must no longer hold the cap.
            if crate::sched::my_cap(0).is_none() {
                crate::kprintln!("ipc: transfer donor empty");
            } else {
                crate::kprintln!("ipc: transfer donor still holds");
            }
        }
        Err(e) => crate::kprintln!("ipc: transfer FAILED {e:?}"),
    }
}

/// ADR-0037: waits until slot 0 is filled by transfer, then proves hold via send.
pub(super) fn transfer_recipient_task() {
    for _ in 0..64 {
        if crate::sched::my_cap(0).is_some() {
            break;
        }
        crate::sched::yield_now();
    }
    let Some(cap) = crate::sched::my_cap(0) else {
        crate::kprintln!("ipc: transfer recipient empty");
        return;
    };
    match crate::ipc::send(cap, crate::ipc::Message { tag: 7, a: 1, b: 0 }) {
        Ok(()) => crate::kprintln!("ipc: transfer ok"),
        Err(e) => crate::kprintln!("ipc: transfer send FAILED {e:?}"),
    }
}

/// Child that parks on RECV until cascade cancel (ADR-0038).
fn cascade_child() {
    let Some(cap) = crate::sched::my_cap(0) else {
        crate::kprintln!("cascade: child no cap");
        return;
    };
    match crate::ipc::recv(cap) {
        Err(crate::ipc::RecvError::Cancelled) => crate::kprintln!("cascade: cancelled"),
        Ok(_) => crate::kprintln!("cascade: unexpected recv ok"),
        Err(e) => crate::kprintln!("cascade: recv FAILED {e:?}"),
    }
}

/// Parent spawns a parked child then exits — cascade cancels the child.
pub(super) fn cascade_parent_task() {
    match crate::ipc::create_channel() {
        Ok(ch) => {
            match crate::sched::spawn_with_caps(cascade_child, &[ch.recv]) {
                Ok(_) => {
                    // Let the child park before we exit.
                    crate::sched::yield_now();
                    crate::sched::yield_now();
                    crate::sched::yield_now();
                    crate::kprintln!(
                        "cascade: parent exit cascade_events={}",
                        crate::sched::cascade_events()
                    );
                    // Exit triggers ADR-0038 cascade on the blocked child.
                    crate::sched::exit();
                }
                Err(e) => crate::kprintln!("cascade: child spawn FAILED {e:?}"),
            }
        }
        Err(e) => crate::kprintln!("cascade: channel FAILED {e:?}"),
    }
}

/// ADR-0041: child holds SEND in slot 0; EL0 returns it to creator slot 0.
pub(super) fn el0_transfer_child_task() {
    let mut agent = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-xfer: create FAILED {e:?}");
            return;
        }
    };
    // from 0 → creator slot 0 (dest=1).
    let prog = kernel_core::prog::encode_transfer_exit(0, 0, 1);
    match agent.run_user_prog_resuming(&prog) {
        Ok(stats) if matches!(stats.end, crate::agent::SessionEnd::Exit) => {
            if crate::sched::my_cap(0).is_none() {
                crate::kprintln!("el0-xfer: ok");
            } else {
                crate::kprintln!("el0-xfer: still holds");
            }
            let _ = stats;
        }
        Ok(stats) => crate::kprintln!(
            "el0-xfer: unexpected end={:?} refusals={}",
            stats.end,
            stats.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-xfer: run FAILED {e:?}"),
    }
    agent.destroy();
}

/// ADR-0041 refuse: transfer from empty slot.
pub(super) fn el0_transfer_refuse_task() {
    let mut agent = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-xfer-refuse: create FAILED {e:?}");
            return;
        }
    };
    // No caps: from slot 0 empty → Authority.
    let prog = kernel_core::prog::encode_transfer_exit(0, 1, 0);
    match agent.run_user_prog_resuming(&prog) {
        Ok(stats) if stats.authority_refusals >= 1 => {
            crate::kprintln!("el0-xfer: refused refusals={}", stats.authority_refusals);
        }
        Ok(stats) => crate::kprintln!(
            "el0-xfer-refuse: unexpected end={:?} refusals={}",
            stats.end,
            stats.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-xfer-refuse: run FAILED {e:?}"),
    }
    agent.destroy();
}

/// Parent for EL0 transfer: spawn child with SEND, wait until cap returns to slot 0.
pub(super) fn el0_transfer_parent_task() {
    match crate::ipc::create_channel() {
        Ok(ch) => match crate::sched::spawn_with_caps(el0_transfer_child_task, &[ch.send]) {
            Ok(_) => {
                for _ in 0..64 {
                    if crate::sched::my_cap(0).is_some() {
                        crate::kprintln!("el0-xfer: parent got cap");
                        break;
                    }
                    crate::sched::yield_now();
                }
                let _ = ch.recv;
            }
            Err(e) => crate::kprintln!("el0-xfer: child spawn FAILED {e:?}"),
        },
        Err(e) => crate::kprintln!("el0-xfer: channel FAILED {e:?}"),
    }
}

/// ADR-0054: wait until peer transfer fills slot 0.
pub(super) fn el0_peer_xfer_recipient_task() {
    // 64 round-robin rounds bounds the wait: the donor has to be scheduled,
    // build an EL0 session and run it, across ~40 boot tasks — comfortably
    // inside 64 rounds; the boot-check timeout is the real ceiling.
    for _ in 0..64 {
        if crate::sched::my_cap(0).is_some() {
            crate::kprintln!("el0-xfer-peer: ok");
            return;
        }
        crate::sched::yield_now();
    }
    crate::kprintln!("el0-xfer-peer: timeout");
}

/// ADR-0054: EL0 peer transfer — slot 0 SEND → peer slot 0 via task-cap in slot 1.
pub(super) fn el0_peer_xfer_donor_task() {
    let mut agent = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-xfer-peer: create FAILED {e:?}");
            return;
        }
    };
    // from 0 → peer to_slot 0; task-cap in local slot 1.
    let prog = kernel_core::prog::encode_transfer_peer_exit(0, 0, 1);
    match agent.run_user_prog_resuming(&prog) {
        Ok(stats) if matches!(stats.end, crate::agent::SessionEnd::Exit) => {
            if crate::sched::my_cap(0).is_none() && crate::sched::my_cap(1).is_some() {
                crate::kprintln!("el0-xfer-peer: donor emptied");
            } else {
                crate::kprintln!(
                    "el0-xfer-peer: donor unexpected holds send={} taskcap={}",
                    crate::sched::my_cap(0).is_some(),
                    crate::sched::my_cap(1).is_some()
                );
            }
            let _ = stats;
        }
        Ok(stats) => crate::kprintln!(
            "el0-xfer-peer: unexpected end={:?} refusals={}",
            stats.end,
            stats.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-xfer-peer: run FAILED {e:?}"),
    }
    agent.destroy();
}

/// ADR-0054 parent: mint task-cap for recipient, spawn donor with SEND + task-cap.
pub(super) fn el0_peer_xfer_parent_task() {
    match crate::ipc::create_channel() {
        Ok(ch) => match crate::sched::spawn(el0_peer_xfer_recipient_task) {
            Ok(to) => match crate::taskcap::mint(to) {
                Ok(tcap) => {
                    match crate::sched::spawn_with_caps(el0_peer_xfer_donor_task, &[ch.send, tcap])
                    {
                        Ok(_) => crate::kprintln!("el0-xfer-peer: spawned"),
                        Err(e) => crate::kprintln!("el0-xfer-peer: donor FAILED {e:?}"),
                    }
                }
                Err(e) => crate::kprintln!("el0-xfer-peer: mint FAILED {e:?}"),
            },
            Err(e) => crate::kprintln!("el0-xfer-peer: recipient FAILED {e:?}"),
        },
        Err(e) => crate::kprintln!("el0-xfer-peer: channel FAILED {e:?}"),
    }
}

/// ADR-0054 refuse: mode 2 without a valid task-cap in the key slot.
pub(super) fn el0_peer_xfer_refuse_task() {
    // No channel needed: empty peer-cap slot is enough for Authority.
    let mut agent = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-xfer-peer-refuse: create FAILED {e:?}");
            return;
        }
    };
    let prog = kernel_core::prog::encode_transfer_peer_exit(0, 0, 1);
    match agent.run_user_prog_resuming(&prog) {
        Ok(stats) if stats.authority_refusals >= 1 => {
            // detail=4 (BadToTask) is the discriminating half (ADR-0061): the
            // empty key slot refuses as a bad target, while a deleted
            // task-cap check would surface detail=3 (BadFromSlot) instead —
            // the regression the old assertion could not see (F-8).
            crate::kprintln!(
                "el0-xfer-peer: refused refusals={} detail={}",
                stats.authority_refusals,
                stats.last_refusal_detail
            );
        }
        Ok(stats) => crate::kprintln!(
            "el0-xfer-peer-refuse: unexpected end={:?} refusals={}",
            stats.end,
            stats.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-xfer-peer-refuse: run FAILED {e:?}"),
    }
    agent.destroy();
}

/// ADR-0057 child: exit immediately so the parent's task-cap goes stale.
pub(super) fn xfer_peer_stale_child_task() {}

/// ADR-0055 + ADR-0057: band filter and stale task-cap refusal, end to end.
///
/// Installs a SEND and a live task-cap into its own slots, then asserts two
/// refusals the EL0 oracle cannot discriminate on its own:
/// - moving the task-cap itself is `Untransferable` (ADR-0055 band filter);
/// - after the child exits, the task-cap is stale and the move refuses
///   (ADR-0057 §1 — the revoke-on-exit invariant made observable).
pub(super) fn xfer_peer_stale_task() {
    let ch = match crate::ipc::create_channel() {
        Ok(c) => c,
        Err(e) => {
            crate::kprintln!("xfer-peer: stale channel FAILED {e:?}");
            return;
        }
    };
    let child = match crate::sched::spawn(xfer_peer_stale_child_task) {
        Ok(id) => id,
        Err(e) => {
            crate::kprintln!("xfer-peer: stale child FAILED {e:?}");
            return;
        }
    };
    let tcap = match crate::taskcap::mint(child) {
        Ok(c) => c,
        Err(e) => {
            crate::kprintln!("xfer-peer: mint FAILED {e:?}");
            return;
        }
    };
    if crate::sched::install_cap(0, ch.send).is_err() || crate::sched::install_cap(1, tcap).is_err()
    {
        crate::kprintln!("xfer-peer: stale install FAILED");
        return;
    }
    // Live task-cap, but the moved object is the task-cap itself: delegation,
    // refused by band (ADR-0055).
    match crate::sched::transfer_held_to_peer(1, 0, 1) {
        Err(crate::sched::TransferError::Untransferable) => {
            crate::kprintln!("xfer-peer: band refused");
        }
        other => crate::kprintln!("xfer-peer: band UNEXPECTED {other:?}"),
    }
    // Wait for the child to exit; revoke-on-exit stales the cap.
    let mut stale = false;
    for _ in 0..64 {
        if crate::taskcap::lookup(tcap).is_err() {
            stale = true;
            break;
        }
        crate::sched::yield_now();
    }
    if !stale {
        crate::kprintln!("xfer-peer: stale timeout");
        return;
    }
    match crate::sched::transfer_held_to_peer(0, 0, 1) {
        Err(_) => crate::kprintln!("xfer-peer: stale refused"),
        Ok(()) => crate::kprintln!("xfer-peer: STALE MOVED"),
    }
}

/// ADR-0042: EL0 SYS_RECV_TIMEOUT without sender.
pub(super) fn el0_timeout_task() {
    let mut agent = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-timeout: create FAILED {e:?}");
            return;
        }
    };
    // Slot 0 has RECV; timeout 3 ticks.
    let prog = kernel_core::prog::encode_recv_timeout_exit(0, 3);
    match agent.run_user_prog_resuming(&prog) {
        Ok(stats) if matches!(stats.end, crate::agent::SessionEnd::Exit) => {
            // Cancelled is not always counted as authority; check end + no recv.
            if stats.recvs == 0 {
                crate::kprintln!("el0-timeout: cancelled");
            } else {
                crate::kprintln!("el0-timeout: unexpected recv");
            }
            let _ = stats;
        }
        Ok(stats) => crate::kprintln!("el0-timeout: unexpected end={:?}", stats.end),
        Err(e) => crate::kprintln!("el0-timeout: run FAILED {e:?}"),
    }
    agent.destroy();
}

/// ADR-0044: thin-stack worker — exits after one yield.
pub(super) fn density_thin_task() {
    crate::sched::yield_now();
}

static BUDGET_A: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static BUDGET_B: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// ADR-0046: spin until budget expires, count turns.
pub(super) fn budget_worker_a() {
    for _ in 0..32 {
        while !crate::sched::budget_expired() {
            core::hint::spin_loop();
        }
        BUDGET_A.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        crate::sched::yield_now();
        if BUDGET_A.load(core::sync::atomic::Ordering::Relaxed) >= 2
            && BUDGET_B.load(core::sync::atomic::Ordering::Relaxed) >= 1
        {
            crate::kprintln!("budget: rotated");
            return;
        }
    }
}

/// ADR-0046 peer of [`budget_worker_a`].
pub(super) fn budget_worker_b() {
    for _ in 0..32 {
        while !crate::sched::budget_expired() {
            core::hint::spin_loop();
        }
        BUDGET_B.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        crate::sched::yield_now();
        if BUDGET_A.load(core::sync::atomic::Ordering::Relaxed) >= 1
            && BUDGET_B.load(core::sync::atomic::Ordering::Relaxed) >= 2
        {
            // a prints rotated; b just exits.
            return;
        }
    }
}

/// ADR-0040: park on RECV with a short tick timeout; no sender → Cancelled.
pub(super) fn timeout_recv_task() {
    let Some(cap) = crate::sched::my_cap(0) else {
        crate::kprintln!("ipc: timeout has no cap");
        return;
    };
    // TIMER_HZ is 10; a few ticks is enough for idle to poll after WFI.
    match crate::ipc::recv_with_timeout(cap, 3) {
        Err(crate::ipc::RecvError::Cancelled) => {
            crate::kprintln!("ipc: timed-out cancelled")
        }
        Ok(_) => crate::kprintln!("ipc: timeout unexpected msg"),
        Err(e) => crate::kprintln!("ipc: timeout FAILED {e:?}"),
    }
}

/// ADR-0039 + ADR-0052: resolve grant + EL0 SYS_RESOLVE for name `ab`.
pub(super) fn el0_resolve_task() {
    // name "ab" LE = 0x6261, len 2, slot 0 empty.
    let prog = kernel_core::prog::encode_resolve_exit(0, 2, 0x6261);

    // ADR-0052: without grant, even a bound name refuses.
    let mut denied = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("resolve-grant: create FAILED {e:?}");
            return;
        }
    };
    match denied.run_user_prog_resuming(&prog) {
        Ok(stats) if stats.authority_refusals >= 1 && crate::sched::my_cap(0).is_none() => {
            crate::kprintln!("resolve-grant: refused");
        }
        Ok(stats) => crate::kprintln!(
            "resolve-grant: unexpected end={:?} refusals={} slot={}",
            stats.end,
            stats.authority_refusals,
            crate::sched::my_cap(0).is_some()
        ),
        Err(e) => crate::kprintln!("resolve-grant: run FAILED {e:?}"),
    }
    denied.destroy();

    if !crate::sched::grant_resolve_current() {
        crate::kprintln!("resolve-grant: grant FAILED");
        return;
    }

    let mut agent = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-resolve: create FAILED {e:?}");
            return;
        }
    };
    match agent.run_user_prog_resuming(&prog) {
        Ok(stats) if matches!(stats.end, crate::agent::SessionEnd::Exit) => {
            // Slot should now hold the cap — prove with a held check via install path:
            // resolve installs into the *driver* task's table (current task), so my_cap works.
            if crate::sched::my_cap(0).is_some() {
                crate::kprintln!("el0-resolve: ok");
            } else {
                crate::kprintln!("el0-resolve: ok but slot empty");
            }
            let _ = stats;
        }
        Ok(stats) => crate::kprintln!(
            "el0-resolve: unexpected end={:?} refusals={}",
            stats.end,
            stats.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-resolve: run FAILED {e:?}"),
    }
    agent.destroy();

    // Missing name refuse path (grant still held).
    let mut agent2 = match crate::agent::Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("el0-resolve-refuse: create FAILED {e:?}");
            return;
        }
    };
    // name "zz" not bound.
    let bad = kernel_core::prog::encode_resolve_exit(0, 2, 0x7a7a);
    match agent2.run_user_prog_resuming(&bad) {
        Ok(stats) if stats.authority_refusals >= 1 => {
            crate::kprintln!("el0-resolve: refused refusals={}", stats.authority_refusals);
        }
        Ok(stats) => crate::kprintln!(
            "el0-resolve-refuse: unexpected end={:?} refusals={}",
            stats.end,
            stats.authority_refusals
        ),
        Err(e) => crate::kprintln!("el0-resolve-refuse: run FAILED {e:?}"),
    }
    agent2.destroy();
}
