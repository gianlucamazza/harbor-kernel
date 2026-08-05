//! Kernel bring-up: the ordered sequence that takes the machine from `_start`
//! to a running console, and nothing else.
//!
//! What the machine then *does* lives in [`console_loop`]; the hardware gates
//! used to debug the M1 interrupt path live in [`selftest`], behind the
//! `bringup` feature. Keeping the three apart matters because only this file
//! has to be read in order: every line here depends on the ones above it.

mod console_loop;
#[cfg(feature = "bringup")]
mod selftest;

use crate::agent::{self, Agent};
use crate::arch::{bootinfo, cpu, el0, exception, mmu, timer};
use kernel_core::layout::Region;
use kernel_core::paging::{MemKind, Perms};
use kernel_core::syscall::{self, Syscall};

use crate::bsp::board;
use crate::console;
use crate::drivers::pl011::Pl011;
use crate::irq;
use crate::mm;
use crate::println;

/// Timer rate (IRQ ticks per second).
const TIMER_HZ: u32 = 10;

/// Kernel heap size, clamped to the identity-mapped RAM window.
const HEAP_SIZE: usize = 64 * 1024 * 1024;

/// Page tables that must remain free once the kernel map is built.
///
/// Every spawned task unmaps a guard page, and a guard page that lands in a
/// 2 MiB block not yet split costs one arena table — which the arena never
/// gets back (ADR-0005). So the worst case is one per spawnable task, plus the
/// device tree's, plus margin.
///
/// Derived from [`sched::MAX_TASKS`] rather than written down: the constant
/// used to say "`MAX_TASKS` is 4" while the scheduler said 12, so the reserve
/// under-counted by 3× and late spawns silently lost their guard page to
/// `OutOfTables`.
/// `MAX_TASKS` counts idle, which runs on the `link.ld` bootstrap stack and
/// never spawns, so the spawnable worst case is one less. The arena size that
/// has to cover this lives in `link.ld`; the boot-time refusal below is what
/// ties the two together, and it names `PAGE_TABLE_ARENA_SIZE` when it fires.
const MIN_SPARE_TABLES: usize = (crate::sched::MAX_TASKS - 1) + 1 + 2;

/// Stop the boot, having said why, when the kernel map could not be established.
///
/// The early map from `boot.s` is RWX across three gigabytes by construction —
/// it exists only to give the first Rust code memory attributes. Everything the
/// kernel claims about itself, W^X and the guard page both, arrives with
/// `mmu::activate`. Continuing without it would hand an interactive console to a
/// machine with no memory protection at all, having said so once in a line that
/// scrolls past in a second.
///
/// A boot that cannot establish its invariants has not degraded, it has failed.
/// Halting is also the honest signal: silence after this message is easier to
/// notice, and harder to ignore, than a prompt that works.
fn refuse_to_boot(uart: &mut Pl011, reason: core::fmt::Arguments<'_>) -> ! {
    println!(uart, "BOOT REFUSED: {reason}");
    println!(
        uart,
        "the early map is RWX — no W^X, no guard page. Halting rather than \
         offering a console on an unprotected machine."
    );
    cpu::halt()
}

/// Full kernel bring-up; never returns.
pub fn run() -> ! {
    // SAFETY: core 0; DAIF still masked from `boot.s`; nothing else has run.
    let Some(mut uart) = (unsafe { console::acquire() }) else {
        // Unreachable in practice — this is the first claim — but there is no
        // console to report it on, so park rather than pretend.
        cpu::halt()
    };

    println!(uart, "Harbor: hello");
    println!(
        uart,
        "EL1 · W^X map · heap · timer + UART RX IRQ · WFI idle"
    );

    exception::init();

    // Inspect what the firmware handed us while every physical address is
    // still readable, and cache the answer. Everything the BSP hard-codes —
    // RAM size, UART clock, peripheral base — is in that blob; parsing it is
    // future work, but the pointer is unrecoverable once lost.
    //
    // SAFETY: the coarse early map is active and this runs once.
    unsafe {
        bootinfo::survey();
    }
    match bootinfo::device_tree() {
        Some(dtb) => println!(uart, "DTB at {dtb:#x}"),
        None => println!(
            uart,
            "no DTB (x0 was {:#x}); board constants are compiled in",
            bootinfo::dtb_address()
        ),
    }

    // No `CPACR_EL1.FPEN` here on purpose: the kernel is built for
    // `aarch64-unknown-none-softfloat`, so it contains no FP/SIMD at all
    // (enforced by `make no-simd`). Leaving FPEN clear means any future stray
    // FP instruction traps loudly instead of silently corrupting the IRQ path,
    // whose trap frame saves no q registers.

    // Heap and frame-pool bounds are decided before the map is built: both are
    // mapped regions (ADR-0012: named pool, not “rest of RAM”).
    let heap_end = (mm::heap_start() + HEAP_SIZE).min(board::memmap::IDENTITY_RAM_END);
    let (frame_base, frame_end) = match mm::frames::range_after_heap(heap_end) {
        Some(range) => range,
        None => refuse_to_boot(
            &mut uart,
            format_args!(
                "frame pool does not fit after heap at {heap_end:#x} \
                 (need {} B under IDENTITY_RAM_END)",
                board::memmap::FRAME_POOL_BYTES
            ),
        ),
    };
    let mut region_buffer = [mm::layout::empty_region(); mm::layout::MAX_REGIONS];
    let regions =
        match mm::layout::kernel_regions(heap_end as u64, frame_end as u64, &mut region_buffer) {
            Ok(regions) => regions,
            Err(error) => {
                // The layout itself is inconsistent — overlapping regions, a
                // mapped guard page, a W+X region. Mapping it would produce
                // something that boots and protects nothing.
                refuse_to_boot(&mut uart, format_args!("layout invalid: {error:?}"))
            }
        };

    // Swap the coarse early map for the real one. On failure the early map
    // stays active, so the report still reaches the console — and then the boot
    // stops, because that map protects nothing.
    // SAFETY: single core, IRQs masked, early map active.
    if let Err((error, region)) = unsafe { mmu::activate(regions) } {
        refuse_to_boot(&mut uart, format_args!("could not map {region}: {error:?}"))
    }

    // `activate` returning `Ok` is not the same as the MMU being on. The claim
    // printed below is about the hardware, so it is read back from `SCTLR_EL1`
    // rather than inferred from the path that just ran.
    if !mmu::is_enabled() {
        refuse_to_boot(
            &mut uart,
            format_args!("activate reported success but SCTLR_EL1.M is clear"),
        )
    }

    println!(
        uart,
        "MMU on  (W^X, guard page at {:#x}, {} B of table arena left)",
        mm::layout::guard_page(),
        mmu::tables_remaining()
    );

    // The kernel map deliberately covers far less than the early one, so the
    // device-tree blob is now outside it. Map it back in: it is the first
    // region whose address only the firmware knows, which is exactly what
    // `mmu::map` exists for.
    if let Some((base, len)) = bootinfo::device_tree_pages() {
        let region = Region {
            base,
            len,
            kind: MemKind::NormalWb,
            // Read-only: nothing in this kernel writes the blob, and a device
            // tree that the kernel can modify is a device tree nobody can trust.
            perms: Perms::RO,
            name: "device tree",
        };
        // SAFETY: kernel map active, IRQs masked; the range is firmware-owned
        // RAM outside every other region.
        match unsafe { mmu::map(&region) } {
            Ok(()) => println!(uart, "DTB mapped: {len} bytes at {base:#x}"),
            Err(error) => println!(uart, "DTB map FAILED: {error:?}"),
        }
    }

    // Every later spawn may cost a table (guard unmap splits a 2 MiB heap
    // block; the arena never frees). Check *after* the DTB map so the reserve
    // is what spawn actually sees — counting before that under-states the cost.
    if mmu::tables_free() < MIN_SPARE_TABLES {
        refuse_to_boot(
            &mut uart,
            format_args!(
                "table arena nearly exhausted: {} tables left, need {MIN_SPARE_TABLES} \
                 (raise PAGE_TABLE_ARENA_SIZE in link.ld)",
                mmu::tables_free()
            ),
        )
    }

    // SAFETY: the heap region was just mapped as Normal memory.
    let heap_ok = unsafe { mm::init_heap(heap_end) };
    if heap_ok {
        println!(uart, "heap remaining = {} bytes", mm::heap_remaining());
    } else {
        println!(uart, "heap UNAVAILABLE (empty region)");
    }

    // ADR-0012 S1: named phys frame pool (identity-mapped with the kernel map).
    // SAFETY: `frame pool` region was mapped RW Normal; exclusive of the heap.
    let frames_ok = unsafe { mm::frames::init(frame_base, frame_end) };
    if frames_ok {
        println!(
            uart,
            "frames: {} free / {}  base={frame_base:#x}  ({} KiB pool)",
            mm::frames::free_count(),
            mm::frames::capacity(),
            board::memmap::FRAME_POOL_BYTES / 1024
        );
        // M5 S2/S3: AS create → prepare (kernel clone + user stack) → EL0 probes.
        m5_aspace_and_el0_smoke(&mut uart);
    } else {
        println!(uart, "frames: UNAVAILABLE");
    }

    // SoC RNG200: after MMU (Device attributes), before IRQs. On-die block,
    // same class as PL011 — always brought up. Logical failure (timeout /
    // health) is soft: one line, boot continues. Never refuse boot for RNG.
    // SAFETY: single core; RNG200 window not otherwise claimed.
    match unsafe { board::rng::init() } {
        Ok(rng) => match rng.try_word() {
            Ok(Some(word)) => println!(uart, "rng200: ok word={word:#010x}"),
            Ok(None) => {
                let mut words = [0u32; 1];
                match rng.read_words(&mut words) {
                    Ok(1) => println!(uart, "rng200: ok word={:#010x}", words[0]),
                    Ok(_) => println!(uart, "rng200: ok (FIFO empty after warm-up)"),
                    Err(error) => println!(uart, "rng200: warm-up ok, read FAILED: {error:?}"),
                }
            }
            Err(error) => println!(uart, "rng200: read FAILED: {error:?}"),
        },
        Err(error) => println!(uart, "rng200: unavailable ({error:?})"),
    }

    // Optional SPI TFT (ADR-0009): SPI0 + ILI9486 init + solid fill so the
    // glass is not left white. After MMU; before IRQs. Handle stays installed.
    #[cfg(feature = "debug-display")]
    {
        // SAFETY: single core; GPIO/SPI0 not otherwise claimed.
        match unsafe { board::display::init_and_panel() } {
            Ok(spi) => {
                let cdiv = spi.cdiv();
                let bit_hz = spi.bit_hz();
                board::display::install(spi);
                println!(
                    uart,
                    "display: ILI9486 up  cdiv={cdiv}  bit_clk={bit_hz} Hz  status"
                );
                // Status surface: structured slots (not a serial mirror).
                crate::status::show_boot_after_display(cdiv, bit_hz, timer::frequency_hz());
            }
            Err(error) => println!(uart, "display: init FAILED: {error:?}"),
        }
    }

    // SAFETY: single-core; exclusive GIC ownership.
    let interrupts_bound = match unsafe { board::irq::init(TIMER_HZ) } {
        Ok(()) => true,
        Err(error) => {
            println!(uart, "IRQ bind FAILED: {error:?}");
            false
        }
    };
    cpu::sync_pipeline();

    println!(
        uart,
        "CNTFRQ={} Hz  timer={} Hz  PPI={}",
        timer::frequency_hz(),
        TIMER_HZ,
        board::irq::TIMER_IRQ
    );

    // Hardware gates, only when built with `--features bringup`.
    #[cfg(feature = "bringup")]
    if !selftest::run(&mut uart) {
        println!(uart, "SELFTEST FAIL — soft console (IRQs masked)");
        selftest::soft_console(&mut uart);
    }

    // No further handlers are registered: freeze the dispatch table so the IRQ
    // path reads state nothing can mutate under it.
    irq::seal();

    // Arm PL011 RX IRQ into the console ring (GIC line already enabled).
    // SAFETY: exclusive console; IRQs still masked.
    if interrupts_bound {
        unsafe {
            console::enable_rx_irq(&uart);
        }
    }

    // Unmask only if the lines are actually bound. Enabling interrupts after a
    // failed bind arms a timer whose handler was never registered: the IRQ
    // fires, the dispatcher acknowledges it with nothing to run, and the
    // console looks alive while the tick counter never moves — a failure that
    // reads as a subtler bug than the one it is.
    if interrupts_bound {
        timer::on_interrupt();
        cpu::sync_pipeline();
        cpu::irq_enable();
        cpu::sync_pipeline();
        println!(uart, "IRQs enabled (timer + UART RX)");
        println!(uart, "idle: WFI when no RX/tick work");
    } else {
        println!(uart, "interrupts stay masked — console is output only");
    }

    console_loop::heap_check(&mut uart);

    // Shared TX for idle + worker tasks (cooperative only; serialized in with_tx).
    console::install_tx(uart);
    crate::sched::init();

    match crate::sched::spawn(demo_task_a) {
        Ok(_) => crate::kprintln!("sched: spawned task-a"),
        Err(e) => crate::kprintln!("sched: spawn task-a FAILED {e:?}"),
    }
    match crate::sched::spawn(demo_task_b) {
        Ok(_) => crate::kprintln!("sched: spawned task-b"),
        Err(e) => crate::kprintln!("sched: spawn task-b FAILED {e:?}"),
    }

    // M5-P1/P2: EL0 from a scheduled task + SVC decode.
    match crate::sched::spawn(el0_scheduled_task) {
        Ok(_) => crate::kprintln!("sched: spawned el0-task"),
        Err(e) => crate::kprintln!("sched: spawn el0-task FAILED {e:?}"),
    }

    // M6 v1: PL011 page-only agent (ADR-0013); destroy = kill.
    match crate::sched::spawn(pl011_agent_task) {
        Ok(_) => crate::kprintln!("sched: spawned pl011-agent"),
        Err(e) => crate::kprintln!("sched: spawn pl011-agent FAILED {e:?}"),
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

    // Deliberate fault, last so the demo tasks are alive when it runs: the
    // probe must overflow its own guard while a peer stack exists, or it cannot
    // show that the fault landed there *instead of* in the peer (M3 done-when).
    #[cfg(feature = "bringup")]
    match crate::sched::spawn(selftest::guard_probe_task) {
        Ok(_) => crate::kprintln!("sched: spawned guard probe"),
        Err(e) => crate::kprintln!("sched: spawn guard probe FAILED {e:?}"),
    }

    // What the spawns cost the arena. Printed here rather than tracked in the
    // idle loop because spawning is a boot-time act today; when tasks come and
    // go at runtime this number is the one that has to be watched, and the
    // counter already exists for that.
    crate::kprintln!(
        "arena: {} splits, {} tables free",
        mmu::splits(),
        mmu::tables_free()
    );

    // Idle body — never returns (ADR-0006).
    console_loop::run()
}

/// Bit pattern of a valid send cap the forger does **not** hold (M4 refuse).
static IPC_FORGE_RAW: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// M5: prepare + SVC/fault probes + dual-AS teardown (bootstrap, before sched).
fn m5_aspace_and_el0_smoke(uart: &mut Pl011) {
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
fn el0_scheduled_task() {
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
fn pl011_agent_task() {
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
fn demo_task_a() {
    for i in 0..4 {
        crate::kprintln!("task-a {i}");
        crate::sched::yield_now();
    }
}

fn demo_task_b() {
    for i in 0..4 {
        crate::kprintln!("task-b {i}");
        crate::sched::yield_now();
    }
}

/// M4: holds recv cap only; blocks until sender posts.
fn ipc_receiver() {
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
fn ipc_sender() {
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
fn ipc_forger() {
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
        Err(_) => crate::kprintln!("ipc: refuse count={}", crate::ipc::refused_count()),
    }
}
