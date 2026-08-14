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
use crate::time;
use kernel_core::paging::Perms;
use kernel_core::syscall::{self, Syscall};

/// Guest time a **cross-core** oracle wait is given, in timer ticks (10 Hz).
///
/// A wait for another core's progress cannot be bounded in yields. The yields
/// are this core's, and how many of them fit before the other core makes its
/// next step is a property of the host's scheduler, not of the kernel: under
/// TCG the vCPU threads are multiplexed, and CPU 0 can spin through thousands
/// of cheap yields while CPU 1's thread has not been picked once. That is how
/// `preempt-el1-cpu1: spinner exit timeout` reached CI while the same image
/// passed every run on a workstation (ADR-0087).
///
/// Ticks are the guest's own clock, so this bound means the same thing on
/// every host. Same-core waits stay counted in yields: there the budget
/// measures the very core whose progress is in question.
const CROSS_CORE_WAIT_TICKS: u64 = 10;

/// Yield ceiling for a tick-bounded wait, so a stopped tick counter is a
/// failure rather than a hang. Reaching it is a different bug from timing out.
const CROSS_CORE_WAIT_CEILING: u32 = 2_000_000;

/// Yield until `f()` or the guest clock has advanced [`CROSS_CORE_WAIT_TICKS`].
///
/// `true` if the condition held in time.
fn wait_ticks_for(f: impl Fn() -> bool) -> bool {
    let deadline = time::ticks() + CROSS_CORE_WAIT_TICKS;
    for _ in 0..CROSS_CORE_WAIT_CEILING {
        if f() {
            return true;
        }
        if time::ticks() >= deadline {
            return false;
        }
        sched::yield_now();
    }
    false
}

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

/// Submit one console byte in a one-shot EL0 session and return whether the
/// non-blocking send was accepted. The console endpoint is deliberately a
/// bounded mailbox: a multi-SVC program could be preempted between two sends
/// while other agents fill it, so the demo drains the endpoint between
/// individually bounded submissions instead of treating `Full` as success.
fn send_console_once(agent: &mut Agent, byte: u8) -> bool {
    match agent.run_user_prog_resuming(&kernel_core::prog::encode_console_once_exit(
        CONSOLE_SLOT,
        byte,
    )) {
        Ok(stats) => stats.sends == 1 && matches!(stats.end, agent::SessionEnd::Exit),
        Err(error) => {
            crate::kprintln!("el0-task: console one-shot FAILED {error:?}");
            false
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

    // Console endpoint: two bounded SYS_SEND submissions, each drained by the
    // EL1 server before the next one. This exercises accepted sends without
    // making the oracle depend on mailbox occupancy or scheduler interleaving.
    let mut console_sends = 0;
    for byte in [b'H', b'!'] {
        drain_console_if_held();
        if send_console_once(&mut agent, byte) {
            console_sends += 1;
        }
    }
    if console_sends == 2 {
        drain_console_if_held();
        crate::kprintln!("el0-task: console sends=2");
    } else {
        crate::kprintln!("el0-task: console unexpected sends={console_sends}");
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
    let id = kernel_core::runqueue::TaskId::from_raw(raw);
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

/// ADR-0090 / K10 residual: **Running** EL1 body that never parks on IPC —
/// only force-exit stops it. Uses WFI (not yield-spam) so peer demos and the
/// tick reporter keep CPU. Agent sessions still observe force-exit in
/// `run_user_prog_resuming` (`SessionEnd::Forced`).
pub(super) fn force_kill_child() {
    loop {
        if crate::sched::take_force_exit() {
            crate::kprintln!("force-kill: child forced");
            return;
        }
        // Timer / resched IPI wakes; supervisor_force_exit requests resched.
        crate::arch::cpu::wait_for_interrupt();
    }
}

/// ADR-0090: spawn Running child, force-exit, wait for slot reclaim.
pub(super) fn force_kill_supervisor() {
    let Ok(id) = crate::sched::spawn(force_kill_child) else {
        crate::kprintln!("force-kill: spawn FAILED");
        return;
    };
    // Let the child run (Ready → Running).
    crate::sched::yield_now();
    crate::sched::yield_now();
    match crate::sched::supervisor_force_exit(id) {
        Ok(()) => crate::kprintln!(
            "force-kill: requested events={}",
            crate::sched::force_exit_events()
        ),
        Err(e) => crate::kprintln!("force-kill: request FAILED {e:?}"),
    }
    for _ in 0..32 {
        // After exit the epoch advances: stale id → `None` (ADR-0062), not Empty.
        match crate::sched::task_state(id) {
            None | Some(kernel_core::tasks::State::Empty) => {
                crate::kprintln!("force-kill: slot empty");
                return;
            }
            Some(_) => crate::sched::yield_now(),
        }
    }
    crate::kprintln!("force-kill: slot still {:?}", crate::sched::task_state(id));
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
            id.slot(),
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
            crate::kprintln!("supervisor: restarted id={}", id2.slot());
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
    let to =
        crate::sched::TaskId::from_raw(TRANSFER_TO.load(core::sync::atomic::Ordering::Relaxed));
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

/// ADR-0086 / K5-S: mini-stack worker — same shape as thin (one yield then exit).
pub(super) fn density_mini_task() {
    crate::sched::yield_now();
}

/// Heartbeat of the EL1 spinner (ADR-0068): advances only while A runs.
static PREEMPT_EL1_HEART: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// Stop word for the spinner; set by the peer once rotation is proven.
static PREEMPT_EL1_STOP: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// ADR-0068: EL1 task that **never yields** — no `yield_now`, no park, no
/// syscall. The only way its peer can run is the IRQ-epilogue preemption.
///
/// This pair replaces the ADR-0046 `budget: rotated` workers: the tick that
/// made their voluntary check true now rotates them first by construction,
/// so the cooperative oracle could never fire again. The claim here is
/// strictly stronger — rotation without cooperation (ADR-0046 reconciled).
pub(super) fn preempt_el1_spinner() {
    use core::sync::atomic::Ordering;
    while PREEMPT_EL1_STOP.load(Ordering::Acquire) == 0 {
        PREEMPT_EL1_HEART.fetch_add(1, Ordering::Relaxed);
        core::hint::spin_loop();
    }
    crate::kprintln!("preempt-el1: spinner exited");
}

/// ADR-0068 peer of [`preempt_el1_spinner`]: counts rounds in which the
/// heartbeat advanced between two of its own turns. The spinner never
/// yields, so a fresh heartbeat means this turn was obtained by preemption.
/// Two rounds prove rotation; the stop word ends the spinner —
/// deterministic termination, no timing guess.
pub(super) fn preempt_el1_peer() {
    use core::sync::atomic::Ordering;
    let mut last = PREEMPT_EL1_HEART.load(Ordering::Relaxed);
    let mut rounds = 0u32;
    for _ in 0..4096 {
        let now = PREEMPT_EL1_HEART.load(Ordering::Relaxed);
        if now != last {
            last = now;
            rounds += 1;
            if rounds >= 2 {
                crate::kprintln!("preempt-el1: rotated");
                PREEMPT_EL1_STOP.store(1, Ordering::Release);
                return;
            }
        }
        crate::sched::yield_now();
    }
    crate::kprintln!("preempt-el1: peer gave up");
    PREEMPT_EL1_STOP.store(1, Ordering::Release);
}

// --- ADR-0079: EL1 preemption on CPU 1 (no console TX from secondary) ---

/// Spinner heartbeat on home=1. Advances only while the non-yielding
/// worker holds the core.
static PREEMPT_EL1_CPU1_HEART: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
/// Stop word for the CPU1 spinner; set by the peer on success/give-up.
static PREEMPT_EL1_CPU1_STOP: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// Peer outcome: 0 = running, 1 = rotated, 2 = gave up. Watched on CPU 0.
static PREEMPT_EL1_CPU1_RESULT: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
/// Spinner observed the stop word and left its loop.
static PREEMPT_EL1_CPU1_EXITED: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// ADR-0079: non-yielding EL1 spinner **pinned to CPU 1**. Never yields;
/// quantum IRQ + EL1 epilogue is the only way the peer can run. No
/// `kprintln` — secondary must not touch the console (ADR-0070).
pub(super) fn preempt_el1_cpu1_spinner() {
    use core::sync::atomic::Ordering;
    while PREEMPT_EL1_CPU1_STOP.load(Ordering::Acquire) == 0 {
        PREEMPT_EL1_CPU1_HEART.fetch_add(1, Ordering::Relaxed);
        core::hint::spin_loop();
    }
    PREEMPT_EL1_CPU1_EXITED.store(1, Ordering::Release);
}

/// ADR-0079 peer of [`preempt_el1_cpu1_spinner`]: same heartbeat-round
/// proof as ADR-0068, but records the result in an atomic for the CPU0
/// watcher (no console TX from core 1).
pub(super) fn preempt_el1_cpu1_peer() {
    use core::sync::atomic::Ordering;
    let mut last = PREEMPT_EL1_CPU1_HEART.load(Ordering::Relaxed);
    let mut rounds = 0u32;
    for _ in 0..4096 {
        let now = PREEMPT_EL1_CPU1_HEART.load(Ordering::Relaxed);
        if now != last {
            last = now;
            rounds += 1;
            if rounds >= 2 {
                PREEMPT_EL1_CPU1_RESULT.store(1, Ordering::Release);
                PREEMPT_EL1_CPU1_STOP.store(1, Ordering::Release);
                return;
            }
        }
        crate::sched::yield_now();
    }
    PREEMPT_EL1_CPU1_RESULT.store(2, Ordering::Release);
    PREEMPT_EL1_CPU1_STOP.store(1, Ordering::Release);
}

/// ADR-0079: primary-side watcher. Prints oracle lines once the CPU1 peer
/// proves rotation (or gives up) and the spinner has exited.
pub(super) fn preempt_el1_cpu1_watch() {
    use core::sync::atomic::Ordering;
    // Both waits are for CPU 1's progress, watched from CPU 0: bounded in
    // guest time, not in this core's yields (ADR-0087).
    if !wait_ticks_for(|| PREEMPT_EL1_CPU1_RESULT.load(Ordering::Acquire) != 0) {
        crate::kprintln!("preempt-el1-cpu1: watch timeout");
        PREEMPT_EL1_CPU1_STOP.store(1, Ordering::Release);
        return;
    }
    match PREEMPT_EL1_CPU1_RESULT.load(Ordering::Acquire) {
        1 => crate::kprintln!("preempt-el1-cpu1: rotated"),
        _ => crate::kprintln!("preempt-el1-cpu1: peer gave up"),
    }
    if wait_ticks_for(|| PREEMPT_EL1_CPU1_EXITED.load(Ordering::Acquire) != 0) {
        crate::kprintln!("preempt-el1-cpu1: spinner exited");
        return;
    }
    crate::kprintln!("preempt-el1-cpu1: spinner exit timeout");
}

// --- ADR-0081: EL0 session + preemption on CPU 1 (no console TX) ---

/// Stop-word PA for the CPU1 EL0 spinner (peer writes; spinner host owns).
static EL0_CPU1_STOP_PA: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static EL0_CPU1_PEER_TURNS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// 0 = running, 1 = rotated, 2 = peer gave up.
static EL0_CPU1_RESULT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// Spinner host finished (session ended / destroy done).
static EL0_CPU1_EXITED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

const EL0_CPU1_STOP_OFF: usize = 0x800;

/// ADR-0081: EL0 spinner host **pinned to CPU 1** — no syscalls; IRQs open;
/// only quantum preemption lets the peer run. Atomics only (no TX).
pub(super) fn el0_cpu1_spinner() {
    use crate::bsp::board::memmap::USER_VA_BASE;
    use core::sync::atomic::Ordering;

    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(_) => {
            EL0_CPU1_RESULT.store(2, Ordering::Release);
            EL0_CPU1_EXITED.store(1, Ordering::Release);
            return;
        }
    };
    if agent
        .aspace()
        .poke_user(EL0_CPU1_STOP_OFF, &0u32.to_le_bytes())
        .is_err()
    {
        agent.destroy();
        EL0_CPU1_RESULT.store(2, Ordering::Release);
        EL0_CPU1_EXITED.store(1, Ordering::Release);
        return;
    }
    let Some(text_pa) = agent.aspace().text_page_phys(0) else {
        agent.destroy();
        EL0_CPU1_RESULT.store(2, Ordering::Release);
        EL0_CPU1_EXITED.store(1, Ordering::Release);
        return;
    };

    let prog = kernel_core::prog::encode_spin_flag_exit(
        (USER_VA_BASE >> 16) as u16,
        EL0_CPU1_STOP_OFF as u16,
    );
    el0::set_entry_irqs_unmasked(sched::current_el0_session());
    EL0_CPU1_STOP_PA.store(text_pa + EL0_CPU1_STOP_OFF, Ordering::Release);
    let _ = agent.run_user_prog_resuming_prep(&prog, || {
        timer::accelerate_next_tick(1);
    });
    EL0_CPU1_STOP_PA.store(0, Ordering::Release);
    el0::set_entry_irqs_masked(sched::current_el0_session());
    agent.destroy();
    EL0_CPU1_EXITED.store(1, Ordering::Release);
}

/// ADR-0081 peer on home=1: counts turns while the spinner window is open.
pub(super) fn el0_cpu1_peer() {
    use core::sync::atomic::Ordering;

    for _ in 0..4096 {
        let stop_pa = EL0_CPU1_STOP_PA.load(Ordering::Acquire);
        if stop_pa != 0 {
            let turns = EL0_CPU1_PEER_TURNS.fetch_add(1, Ordering::Relaxed) + 1;
            if turns >= 2 {
                EL0_CPU1_RESULT.store(1, Ordering::Release);
                // SAFETY: same contract as `preempt_peer_task` stop-word write.
                unsafe { core::ptr::write_volatile(stop_pa as *mut u32, 1) };
                return;
            }
        }
        crate::sched::yield_now();
    }
    EL0_CPU1_RESULT.store(2, Ordering::Release);
}

/// ADR-0081: primary watcher — prints oracle lines for the CPU1 EL0 pair.
pub(super) fn el0_cpu1_watch() {
    use core::sync::atomic::Ordering;
    // Cross-core, so guest time bounds it — see `preempt_el1_cpu1_watch`.
    if !wait_ticks_for(|| EL0_CPU1_RESULT.load(Ordering::Acquire) != 0) {
        crate::kprintln!("preempt-el0-cpu1: watch timeout");
        return;
    }
    match EL0_CPU1_RESULT.load(Ordering::Acquire) {
        1 => crate::kprintln!("preempt-el0-cpu1: rotated"),
        _ => crate::kprintln!("preempt-el0-cpu1: peer gave up"),
    }
    if wait_ticks_for(|| EL0_CPU1_EXITED.load(Ordering::Acquire) != 0) {
        crate::kprintln!("preempt-el0-cpu1: spinner exited");
        return;
    }
    crate::kprintln!("preempt-el0-cpu1: spinner exit timeout");
}

// --- ADR-0083: work steal (victims on CPU0 only; no spawn_on(1)) ---

/// Set when a steal victim observes affinity 1.
static STEAL_RAN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// ADR-0083: EL1 worker admitted on **CPU0 only**. Opts into stealeable,
/// then cooperative yields so a peer victim sits Ready and CPU1 can pull.
pub(super) fn steal_victim() {
    use core::sync::atomic::Ordering;
    crate::sched::mark_current_stealeable();
    for _ in 0..8192 {
        if crate::arch::cpu::affinity() == 1 {
            STEAL_RAN.store(true, Ordering::Release);
            return;
        }
        crate::sched::yield_now();
    }
}

/// ADR-0083: primary watcher — proves steal without `spawn_on(1)`.
/// Not stealeable (console TX stays on CPU0).
pub(super) fn steal_watch() {
    use core::sync::atomic::Ordering;
    crate::sched::mark_current_not_stealeable();
    // The victim runs on CPU 1: cross-core, so guest time bounds the wait
    // (ADR-0087).
    if wait_ticks_for(|| STEAL_RAN.load(Ordering::Acquire)) {
        crate::kprintln!("smp: steal ok");
        return;
    }
    crate::kprintln!("smp: steal timeout");
}

/// Stop-word physical address of the live preempt spinner (ADR-0064).
/// Zero = no spinner window open. Written by [`preempt_agent_task`], consumed
/// by [`preempt_peer_task`].
static PREEMPT_STOP_PA: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static PREEMPT_PEER_TURNS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Byte offset of the ADR-0064 stop word inside text page 0 — past the
/// program, word-aligned, within the page.
const PREEMPT_STOP_OFF: usize = 0x800;

/// ADR-0064: EL0 spinner that makes **no** syscalls — preemption on the IRQ
/// epilogue is the only way it loses the CPU. The peer proves rotation and
/// ends it by writing the stop word.
pub(super) fn preempt_agent_task() {
    use crate::bsp::board::memmap::USER_VA_BASE;

    let mut agent = match Agent::create_prepared() {
        Ok(a) => a,
        Err(e) => {
            crate::kprintln!("preempt: create FAILED {e:?}");
            return;
        }
    };
    // The stop word must start zero; a pool frame's content is not assumed.
    if let Err(e) = agent
        .aspace()
        .poke_user(PREEMPT_STOP_OFF, &0u32.to_le_bytes())
    {
        crate::kprintln!("preempt: stop-word poke FAILED {e:?}");
        agent.destroy();
        return;
    }
    let Some(text_pa) = agent.aspace().text_page_phys(0) else {
        crate::kprintln!("preempt: no text page");
        agent.destroy();
        return;
    };

    let prog = kernel_core::prog::encode_spin_flag_exit(
        (USER_VA_BASE >> 16) as u16,
        PREEMPT_STOP_OFF as u16,
    );
    // EL0 IRQs open, or the tick is claimed at EL1 and no `Irq` outcome — and
    // therefore no preempt safe point — ever arrives.
    el0::set_entry_irqs_unmasked(sched::current_el0_session());
    PREEMPT_STOP_PA.store(
        text_pa + PREEMPT_STOP_OFF,
        core::sync::atomic::Ordering::Release,
    );
    let res = agent.run_user_prog_resuming_prep(&prog, || {
        timer::accelerate_next_tick(1);
    });
    PREEMPT_STOP_PA.store(0, core::sync::atomic::Ordering::Release);
    el0::set_entry_irqs_masked(sched::current_el0_session());

    match res {
        Ok(s) if matches!(s.end, agent::SessionEnd::Exit) => {
            crate::kprintln!("preempt: spinner exited irqs={}", s.irqs);
        }
        Ok(s) => crate::kprintln!("preempt: spinner unexpected end {:?}", s.end),
        Err(e) => crate::kprintln!("preempt: spinner FAILED {e:?}"),
    }
    agent.destroy();
}

/// ADR-0064 peer of [`preempt_agent_task`]: counts its own scheduled turns
/// while the spinner window is open. The spinner's host task never yields, so
/// every turn here means the CPU was taken from it involuntarily. Two turns
/// prove rotation; the oracle line prints and the stop word ends the spinner.
pub(super) fn preempt_peer_task() {
    use core::sync::atomic::Ordering;

    for _ in 0..4096 {
        let stop_pa = PREEMPT_STOP_PA.load(Ordering::Acquire);
        if stop_pa != 0 {
            let turns = PREEMPT_PEER_TURNS.fetch_add(1, Ordering::Relaxed) + 1;
            if turns >= 2 {
                crate::kprintln!("preempt: rotated");
                // SAFETY: `stop_pa` names the stop word inside a text frame
                // the live agent owns, published for exactly this write and
                // cleared when the session ends. The identity alias is Normal
                // WB like the EL0 mapping of the same PA, so the store is
                // coherent with the spinner's loads (same discipline as
                // `poke_user`, minus the I-side maintenance a data word does
                // not need).
                unsafe { core::ptr::write_volatile(stop_pa as *mut u32, 1) };
                return;
            }
        }
        crate::sched::yield_now();
    }
    crate::kprintln!("preempt: peer gave up");
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

/// Spawn every demo the boot oracle asserts on, in the order it asserts them.
///
/// Everything the boot oracle needs, and nothing the product does. Rule 9
/// of `architecture.md` keeps diagnostic scaffolding out of the production
/// surface; `make product-builds` compiles the image without it and refuses
/// an ELF that still carries a demo symbol.
///
/// Order is load-bearing and preserved from `bootstrap::run`: the oracle reads
/// these lines in sequence, and the ADR-0090 force-kill supervisor stays
/// **last** for the reason its own comment gives.
pub(super) fn run_all(console_cap: Option<kernel_core::cap::CapId>) {
    match crate::sched::spawn(demo_task_a) {
        Ok(_) => crate::kprintln!("sched: spawned task-a"),
        Err(e) => crate::kprintln!("sched: spawn task-a FAILED {e:?}"),
    }
    match crate::sched::spawn(demo_task_b) {
        Ok(_) => crate::kprintln!("sched: spawned task-b"),
        Err(e) => crate::kprintln!("sched: spawn task-b FAILED {e:?}"),
    }

    // Slot 0 is left empty for these two on purpose: their programs name
    // `CONSOLE_SLOT` (1), so the table has a hole under it, and an agent that
    // miscounts its own slots is refused rather than served something adjacent.
    let console_caps: [Option<kernel_core::cap::CapId>; 2] = [None, console_cap];
    // M5-P1/P2: EL0 from a scheduled task + SVC decode.
    match crate::sched::spawn_with_slots(el0_scheduled_task, &console_caps) {
        Ok(_) => crate::kprintln!("sched: spawned el0-task"),
        Err(e) => crate::kprintln!("sched: spawn el0-task FAILED {e:?}"),
    }

    // M6 v1: PL011 page-only agent (ADR-0013); destroy = kill.
    match crate::sched::spawn_with_slots(pl011_agent_task, &console_caps) {
        Ok(_) => crate::kprintln!("sched: spawned pl011-agent"),
        Err(e) => crate::kprintln!("sched: spawn pl011-agent FAILED {e:?}"),
    }

    // K9 / ADR-0034: second peripheral agent (RNG200 page map + kill).
    match crate::sched::spawn(rng_agent_task) {
        Ok(_) => crate::kprintln!("sched: spawned rng-agent"),
        Err(e) => crate::kprintln!("sched: spawn rng-agent FAILED {e:?}"),
    }

    // Multi-agent shell: two TCBs, two AS live together, each EL0 once.
    match crate::sched::spawn(agent::concurrent_agent_alpha) {
        Ok(_) => crate::kprintln!("sched: spawned agent-a"),
        Err(e) => crate::kprintln!("sched: spawn agent-a FAILED {e:?}"),
    }
    match crate::sched::spawn(agent::concurrent_agent_beta) {
        Ok(_) => crate::kprintln!("sched: spawned agent-b"),
        Err(e) => crate::kprintln!("sched: spawn agent-b FAILED {e:?}"),
    }

    // M4: mailbox + caps. Message path only — no shared payload static.
    match crate::ipc::create_channel() {
        Ok(ch) => {
            // Forger learns the bit pattern but does not hold the cap in its table.
            IPC_FORGE_RAW.store(ch.send.raw(), core::sync::atomic::Ordering::Relaxed);
            match crate::sched::spawn_with_caps(ipc_receiver, &[ch.recv]) {
                Ok(_) => crate::kprintln!("ipc: spawned receiver"),
                Err(e) => crate::kprintln!("ipc: spawn receiver FAILED {e:?}"),
            }
            match crate::sched::spawn_with_caps(ipc_sender, &[ch.send]) {
                Ok(_) => crate::kprintln!("ipc: spawned sender"),
                Err(e) => crate::kprintln!("ipc: spawn sender FAILED {e:?}"),
            }
            match crate::sched::spawn(ipc_forger) {
                Ok(_) => crate::kprintln!("ipc: spawned forger"),
                Err(e) => crate::kprintln!("ipc: spawn forger FAILED {e:?}"),
            }
        }
        Err(e) => crate::kprintln!("ipc: create_channel FAILED {e:?}"),
    }

    // M7 slice 2: the same exchange, but between two *EL0 agents*, each holding
    // one capability at slot 0 of its own table. A second channel rather than a
    // shared one: two tasks holding two ends of the same mailbox is M4's demo,
    // and reusing it would have the EL1 receiver race the EL0 one for the
    // message.
    match crate::ipc::create_channel() {
        Ok(ch) => {
            // Receiver **first** (ADR-0022 §"gates that catch reversal"). It
            // reaches `SYS_RECV` on an empty mailbox and parks; the sender
            // that follows is what wakes it. Spawned the other way round —
            // as it was until the park existed — the exchange would work
            // whether or not a blocking recv did.
            match crate::sched::spawn_with_slots(el0_ipc_receiver, &[Some(ch.recv), console_cap]) {
                Ok(_) => crate::kprintln!("el0-ipc: spawned receiver"),
                Err(e) => crate::kprintln!("el0-ipc: spawn receiver FAILED {e:?}"),
            }
            match crate::sched::spawn_with_caps(el0_ipc_sender, &[ch.send]) {
                Ok(_) => crate::kprintln!("el0-ipc: spawned sender"),
                Err(e) => crate::kprintln!("el0-ipc: spawn sender FAILED {e:?}"),
            }
        }
        Err(e) => crate::kprintln!("ipc: create_channel FAILED {e:?}"),
    }

    // K1 / ADR-0028 + ADR-0030: mint timer IRQ notification; EL1 wait then
    // EL0 SYS_WAIT_IRQ on the same task (sequential; one waiter per cookie).
    let irq_timer_cap = match crate::irq::cap::mint(1) {
        Ok(c) => {
            crate::kprintln!("irq-cap: minted timer cookie=1");
            Some(c)
        }
        Err(e) => {
            crate::kprintln!("irq-cap: mint FAILED {e:?}");
            None
        }
    };
    match crate::sched::spawn_with_slots(irq_wait_task, &[irq_timer_cap]) {
        Ok(_) => crate::kprintln!("sched: spawned irq-wait"),
        Err(e) => crate::kprintln!("sched: spawn irq-wait FAILED {e:?}"),
    }
    // Empty-slot SYS_WAIT_IRQ must refuse on the good path.
    match crate::sched::spawn(el0_irq_refuse_task) {
        Ok(_) => crate::kprintln!("sched: spawned el0-irq-refuse"),
        Err(e) => crate::kprintln!("sched: spawn el0-irq-refuse FAILED {e:?}"),
    }

    // ADR-0035 / P5: bind a service name to a CapId; resolve and missing.
    match crate::ipc::create_channel() {
        Ok(ch) => {
            match crate::naming::bind(b"svc", ch.send) {
                Ok(()) => match crate::naming::resolve(b"svc") {
                    Ok(c) if c == ch.send => crate::kprintln!("name: resolved"),
                    Ok(_) => crate::kprintln!("name: resolved WRONG cap"),
                    Err(e) => crate::kprintln!("name: resolve FAILED {e:?}"),
                },
                Err(e) => crate::kprintln!("name: bind FAILED {e:?}"),
            }
            match crate::naming::resolve(b"nope") {
                Err(crate::naming::ResolveError::Missing) => {
                    crate::kprintln!("name: missing")
                }
                other => crate::kprintln!("name: missing unexpected {other:?}"),
            }
            let _ = crate::naming::unbind(b"svc");
            // Channel only for the name demo; drop ends.
            let _ = crate::ipc::creator_revoke(ch.send);
        }
        Err(e) => crate::kprintln!("name: channel FAILED {e:?}"),
    }

    // ADR-0036 / P2: on-target put/get of a keyed blob (not host inject).
    match crate::storage::put(b"cfg", b"harbor-p2") {
        Ok(()) => {
            let mut out = [0u8; 16];
            match crate::storage::get(b"cfg", &mut out) {
                Ok(n) if &out[..n] == b"harbor-p2" => crate::kprintln!("store: got"),
                Ok(_) => crate::kprintln!("store: got WRONG payload"),
                Err(e) => crate::kprintln!("store: get FAILED {e:?}"),
            }
        }
        Err(e) => crate::kprintln!("store: put FAILED {e:?}"),
    }
    match crate::storage::get(b"nope", &mut [0u8; 8]) {
        Err(crate::storage::GetError::Missing) => crate::kprintln!("store: missing"),
        other => crate::kprintln!("store: missing unexpected {other:?}"),
    }
    let _ = crate::storage::delete(b"cfg");

    // ADR-0037 / K3 residual: transfer SEND from donor to recipient.
    match crate::ipc::create_channel() {
        Ok(ch) => {
            match crate::sched::spawn(transfer_recipient_task) {
                Ok(to) => {
                    TRANSFER_TO.store(to.to_raw(), core::sync::atomic::Ordering::Relaxed);
                    match crate::sched::spawn_with_caps(transfer_donor_task, &[ch.send]) {
                        Ok(_) => crate::kprintln!("ipc: transfer spawned"),
                        Err(e) => crate::kprintln!("ipc: transfer donor FAILED {e:?}"),
                    }
                }
                Err(e) => crate::kprintln!("ipc: transfer recipient FAILED {e:?}"),
            }
            let _ = ch.recv;
        }
        Err(e) => crate::kprintln!("ipc: transfer channel FAILED {e:?}"),
    }

    // ADR-0038 / K10 residual: parent exits → blocked child cancelled.
    match crate::sched::spawn(cascade_parent_task) {
        Ok(_) => crate::kprintln!("cascade: parent spawned"),
        Err(e) => crate::kprintln!("cascade: parent spawn FAILED {e:?}"),
    }

    // ADR-0039 / P5 residual: bind short name, EL0 resolves into empty slot.
    match crate::ipc::create_channel() {
        Ok(ch) => {
            let _ = crate::naming::bind(b"ab", ch.send);
            match crate::sched::spawn(el0_resolve_task) {
                Ok(_) => crate::kprintln!("el0-resolve: spawned"),
                Err(e) => crate::kprintln!("el0-resolve: spawn FAILED {e:?}"),
            }
            let _ = ch.recv;
        }
        Err(e) => crate::kprintln!("el0-resolve: channel FAILED {e:?}"),
    }

    // ADR-0040 / K2 residual: recv timeout without a sender.
    match crate::ipc::create_channel() {
        Ok(ch) => {
            match crate::sched::spawn_with_caps(timeout_recv_task, &[ch.recv]) {
                Ok(_) => crate::kprintln!("ipc: timeout-recv spawned"),
                Err(e) => crate::kprintln!("ipc: timeout-recv spawn FAILED {e:?}"),
            }
            // Keep send off-task so the waiter is not auto-reaped by K2 hold drop.
            let _ = ch.send;
        }
        Err(e) => crate::kprintln!("ipc: timeout channel FAILED {e:?}"),
    }

    // ADR-0041 / K3 residual: EL0 transfer return-to-creator.
    match crate::sched::spawn(el0_transfer_parent_task) {
        Ok(_) => crate::kprintln!("el0-xfer: parent spawned"),
        Err(e) => crate::kprintln!("el0-xfer: parent spawn FAILED {e:?}"),
    }
    match crate::sched::spawn(el0_transfer_refuse_task) {
        Ok(_) => crate::kprintln!("el0-xfer: refuse spawned"),
        Err(e) => crate::kprintln!("el0-xfer: refuse spawn FAILED {e:?}"),
    }

    // ADR-0054 / K3 residual: EL0 peer transfer via task-cap.
    match crate::sched::spawn(el0_peer_xfer_parent_task) {
        Ok(_) => crate::kprintln!("el0-xfer-peer: parent spawned"),
        Err(e) => crate::kprintln!("el0-xfer-peer: parent spawn FAILED {e:?}"),
    }
    match crate::sched::spawn(el0_peer_xfer_refuse_task) {
        Ok(_) => crate::kprintln!("el0-xfer-peer: refuse spawned"),
        Err(e) => crate::kprintln!("el0-xfer-peer: refuse spawn FAILED {e:?}"),
    }

    // ADR-0055 / ADR-0057: band filter + stale task-cap refusal.
    match crate::sched::spawn(xfer_peer_stale_task) {
        Ok(_) => crate::kprintln!("xfer-peer: stale spawned"),
        Err(e) => crate::kprintln!("xfer-peer: stale spawn FAILED {e:?}"),
    }

    // ADR-0042 / K2 residual: EL0 SYS_RECV_TIMEOUT.
    match crate::ipc::create_channel() {
        Ok(ch) => {
            match crate::sched::spawn_with_caps(el0_timeout_task, &[ch.recv]) {
                Ok(_) => crate::kprintln!("el0-timeout: spawned"),
                Err(e) => crate::kprintln!("el0-timeout: spawn FAILED {e:?}"),
            }
            let _ = ch.send;
        }
        Err(e) => crate::kprintln!("el0-timeout: channel FAILED {e:?}"),
    }

    // ADR-0043 / K9 residual: IRQ-cap device wait is proven sequentially
    // inside irq_wait_task (one waiter per cookie — no concurrent race).

    // ADR-0044 / K5 + ADR-0086 / K5-S: thin + mini density workers.
    // Census is fixed-size: do **not** raise MAX_TASKS for denser demos
    // (ADR-0085). Two of each class keeps the same slot budget as the
    // former three thin workers plus one spare for later oracles.
    {
        let mut n = 0u32;
        for _ in 0..2 {
            match crate::sched::spawn_thin(density_thin_task) {
                Ok(_) => n += 1,
                Err(_) => break,
            }
        }
        let each = kernel_core::density::bytes_per_task(kernel_core::density::StackClass::Thin);
        crate::kprintln!("density: thin n={n} bytes_each={each}");
    }
    {
        let mut n = 0u32;
        for _ in 0..2 {
            match crate::sched::spawn_mini(density_mini_task) {
                Ok(_) => n += 1,
                Err(_) => break,
            }
        }
        let each = kernel_core::density::bytes_per_task(kernel_core::density::StackClass::Mini);
        crate::kprintln!("density: mini n={n} bytes_each={each}");
    }

    // ADR-0066 / P2: durable store on true media. The card's partition
    // table names the store window (type 0x7f); load runs BEFORE any
    // put, so the `boot` counter read below is evidence of the previous
    // boot, not of this one. Every degraded path is one honest line and
    // the boot proceeds with the DRAM-only store (ADR-0045 behavior).
    let durable_media = {
        use kernel_core::mbr;
        // SAFETY: exclusive SDHCI windows; core 0 only.
        match unsafe { crate::bsp::board::sdhci::init() } {
            Ok((sd, host)) => {
                let mut sector0 = [0u8; 512];
                match sd.read_block(0, &mut sector0) {
                    Ok(()) => match mbr::parse(&sector0) {
                        Ok(entries) => match mbr::find_store_partition(&entries) {
                            Some((lba, _sectors)) => Some((sd, lba, host)),
                            None => {
                                crate::kprintln!("durable-media: no-partition (no 0x7f entry)");
                                None
                            }
                        },
                        Err(e) => {
                            crate::kprintln!("durable-media: no-partition ({e:?})");
                            None
                        }
                    },
                    Err(e) => {
                        crate::kprintln!("durable-media: error (mbr read {e:?})");
                        None
                    }
                }
            }
            Err(crate::drivers::sdhci::SdError::NotPresent) => {
                crate::kprintln!("durable-media: absent (NotPresent)");
                None
            }
            Err(crate::drivers::sdhci::SdError::NoCard) => {
                crate::kprintln!("durable-media: no-card (no SDHC/SDXC answered)");
                None
            }
            Err(crate::drivers::sdhci::SdError::Unsupported) => {
                crate::kprintln!("durable-media: unsupported (not SDHC/SDXC)");
                None
            }
            Err(e) => {
                crate::kprintln!("durable-media: error (init {e:?})");
                None
            }
        }
    };

    // Load the winning slot, seed the region, read the counter, print
    // the cross-boot evidence line, then advance the counter.
    let durable_flush = durable_media.and_then(|(sd, lba, host)| {
        let mut loaded = [0u8; kernel_core::durable::REGION_SIZE];
        let winner = match sd.media_load(lba, &mut loaded) {
            Ok(w) => w,
            Err(e) => {
                crate::kprintln!("durable-media: error (load {e:?})");
                return None;
            }
        };
        if winner.is_some() {
            crate::durable::restore(&loaded);
        }
        let mut out = [0u8; 4];
        let prev = match crate::durable::get(b"boot", &mut out) {
            Ok(4) => u32::from_le_bytes(out),
            _ => 0,
        };
        let boot = prev + 1;
        match winner {
            Some((slot, seq)) => crate::kprintln!(
                "durable-media: boot={boot} from=Previous part=0x7f slot={slot:?} seq={seq} host={host}"
            ),
            None => crate::kprintln!(
                "durable-media: boot={boot} from=Fresh part=0x7f slot=- seq=0 host={host}"
            ),
        }
        if let Err(e) = crate::durable::put(b"boot", &boot.to_le_bytes()) {
            crate::kprintln!("durable-media: error (counter put {e:?})");
            return None;
        }
        Some((sd, lba, winner))
    });

    // ADR-0045 / P2 durable: put → get from durable region (no host inject).
    match crate::durable::put(b"cfg", b"persist") {
        Ok(()) => {
            let mut out = [0u8; 16];
            match crate::durable::get(b"cfg", &mut out) {
                Ok(len) if &out[..len] == b"persist" => {
                    crate::kprintln!("durable: reloaded");
                }
                Ok(_) => crate::kprintln!("durable: bad payload"),
                Err(e) => crate::kprintln!("durable: get FAILED {e:?}"),
            }
        }
        Err(e) => crate::kprintln!("durable: put FAILED {e:?}"),
    }

    // ADR-0066: one explicit flush point — snapshot the region, write
    // the opposite slot (header last = commit), then read back.
    if let Some((sd, lba, winner)) = durable_flush {
        let seq = winner.map(|(_, s)| s).unwrap_or(0);
        let snap = crate::durable::snapshot();
        match sd.media_flush(lba, winner.map(|(s, _)| s), seq, &snap) {
            Ok((slot, new_seq)) => {
                crate::kprintln!("durable-media: flushed slot={slot:?} seq={new_seq}");
                match sd.media_verify(lba, slot, new_seq, &snap) {
                    Ok(true) => crate::kprintln!("durable-media: verified"),
                    Ok(false) => crate::kprintln!("durable-media: error (verify mismatch)"),
                    Err(e) => crate::kprintln!("durable-media: error (verify {e:?})"),
                }
            }
            Err(e) => crate::kprintln!("durable-media: error (flush {e:?})"),
        }
    }

    // ADR-0068 / K4: same-EL preemption — an EL1 spinner that never
    // yields loses the CPU on the IRQ epilogue. Replaces the ADR-0046
    // cooperative pair (whose voluntary check the epilogue now wins by
    // construction). Peer first, same discipline as the EL0 demo.
    match crate::sched::spawn_thin(preempt_el1_peer) {
        Ok(_) => match crate::sched::spawn_thin(preempt_el1_spinner) {
            Ok(_) => crate::kprintln!("preempt-el1: workers spawned"),
            Err(e) => crate::kprintln!("preempt-el1: spinner spawn FAILED {e:?}"),
        },
        Err(e) => crate::kprintln!("preempt-el1: peer spawn FAILED {e:?}"),
    }

    // ADR-0079 / K8: same claim on home=1 — local CNTP + EL1 epilogue.
    // Watcher on CPU0 prints (no TX from core 1); peer then spinner on 1.
    match crate::sched::spawn_thin(preempt_el1_cpu1_watch) {
        Ok(_) => match crate::sched::spawn_on(1, preempt_el1_cpu1_peer) {
            Ok(_) => match crate::sched::spawn_on(1, preempt_el1_cpu1_spinner) {
                Ok(_) => crate::kprintln!("preempt-el1-cpu1: workers spawned"),
                Err(e) => crate::kprintln!("preempt-el1-cpu1: spinner spawn FAILED {e:?}"),
            },
            Err(e) => crate::kprintln!("preempt-el1-cpu1: peer spawn FAILED {e:?}"),
        },
        Err(e) => crate::kprintln!("preempt-el1-cpu1: watch spawn FAILED {e:?}"),
    }

    // ADR-0081 / K8: EL0 session + quantum preemption on home=1.
    // Publish is per-CPU; peer then spinner on 1; watcher prints on 0.
    match crate::sched::spawn_thin(el0_cpu1_watch) {
        Ok(_) => match crate::sched::spawn_on(1, el0_cpu1_peer) {
            Ok(_) => match crate::sched::spawn_on(1, el0_cpu1_spinner) {
                Ok(_) => crate::kprintln!("preempt-el0-cpu1: workers spawned"),
                Err(e) => crate::kprintln!("preempt-el0-cpu1: spinner spawn FAILED {e:?}"),
            },
            Err(e) => crate::kprintln!("preempt-el0-cpu1: peer spawn FAILED {e:?}"),
        },
        Err(e) => crate::kprintln!("preempt-el0-cpu1: watch spawn FAILED {e:?}"),
    }

    // ADR-0083 / K8: work steal — all admitted on CPU0; no spawn_on(1).
    // Two cooperative victims so one is Ready while the other runs; CPU1 pulls.
    match crate::sched::spawn_thin(steal_watch) {
        Ok(_) => match crate::sched::spawn_thin(steal_victim) {
            Ok(_) => match crate::sched::spawn_thin(steal_victim) {
                Ok(_) => crate::kprintln!("smp: steal workers spawned"),
                Err(e) => crate::kprintln!("smp: steal victim2 spawn FAILED {e:?}"),
            },
            Err(e) => crate::kprintln!("smp: steal victim spawn FAILED {e:?}"),
        },
        Err(e) => crate::kprintln!("smp: steal watch spawn FAILED {e:?}"),
    }

    // ADR-0064 / K4: IRQ-side preemption of a non-syscalling EL0 spinner.
    // Peer first, so it is already in the rotation when the window opens.
    match crate::sched::spawn_thin(preempt_peer_task) {
        Ok(_) => match crate::sched::spawn(preempt_agent_task) {
            Ok(_) => crate::kprintln!("preempt: tasks spawned"),
            Err(e) => crate::kprintln!("preempt: agent spawn FAILED {e:?}"),
        },
        Err(e) => crate::kprintln!("preempt: peer spawn FAILED {e:?}"),
    }

    // ADR-0032 / K3: a task that holds SEND revokes the channel; bootstrap
    // then proves the stale CapId refuses send (product path, not forged).
    match crate::ipc::create_channel() {
        Ok(ch) => {
            REVOKE_STALE.store(ch.send.raw(), core::sync::atomic::Ordering::Relaxed);
            match crate::sched::spawn_with_caps(revoke_held_task, &[ch.send]) {
                Ok(_) => crate::kprintln!("ipc: revoke-held spawned"),
                Err(e) => crate::kprintln!("ipc: revoke-held spawn FAILED {e:?}"),
            }
        }
        Err(e) => crate::kprintln!("ipc: revoke channel FAILED {e:?}"),
    }

    // ADR-0025: park with no sender, then cancel from a supervisor task.
    // The send capability is dropped — the orphan cannot be woken by IPC.
    match crate::ipc::create_channel() {
        Ok(ch) => {
            match crate::sched::spawn_with_caps(orphan_receiver, &[ch.recv]) {
                Ok(id) => {
                    ORPHAN_TASK.store(id.to_raw(), core::sync::atomic::Ordering::Relaxed);
                    crate::kprintln!("ipc: orphan spawned id={}", id.slot());
                }
                Err(e) => crate::kprintln!("ipc: orphan spawn FAILED {e:?}"),
            }
            // `ch.send` dropped here: nobody holds send.
            match crate::sched::spawn(orphan_reaper) {
                Ok(_) => crate::kprintln!("ipc: reaper spawned"),
                Err(e) => crate::kprintln!("ipc: reaper spawn FAILED {e:?}"),
            }
        }
        Err(e) => crate::kprintln!("ipc: orphan channel FAILED {e:?}"),
    }

    // ADR-0033 / K10: product supervisor reaps a blocked child and restarts.
    match crate::sched::spawn(supervisor_task) {
        Ok(_) => crate::kprintln!("sched: spawned supervisor"),
        Err(e) => crate::kprintln!("sched: spawn supervisor FAILED {e:?}"),
    }

    // ADR-0031 / K2: ephemeral channel — sole SEND holder exits, waiter
    // is auto-cancelled without a supervisor reaper.
    match crate::ipc::create_channel_ephemeral() {
        Ok(ch) => {
            match crate::sched::spawn_with_caps(auto_reap_receiver, &[ch.recv]) {
                Ok(_) => crate::kprintln!("ipc: auto-reap receiver spawned"),
                Err(e) => crate::kprintln!("ipc: auto-reap receiver FAILED {e:?}"),
            }
            match crate::sched::spawn_with_caps(auto_reap_sender, &[ch.send]) {
                Ok(_) => crate::kprintln!("ipc: auto-reap sender spawned"),
                Err(e) => crate::kprintln!("ipc: auto-reap sender FAILED {e:?}"),
            }
        }
        Err(e) => crate::kprintln!("ipc: auto-reap channel FAILED {e:?}"),
    }

    // ADR-0090 / K10 residual: force-exit Running EL0 — **last** among
    // oracle demos so a long EL0 spin cannot interleave frame-pool free
    // counts with `agents: concurrent` (false LEAK).
    match crate::sched::spawn(force_kill_supervisor) {
        Ok(_) => crate::kprintln!("sched: spawned force-kill supervisor"),
        Err(e) => crate::kprintln!("sched: spawn force-kill FAILED {e:?}"),
    }
}
