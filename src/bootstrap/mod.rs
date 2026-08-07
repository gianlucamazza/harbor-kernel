//! Kernel bring-up: the ordered sequence that takes the machine from `_start`
//! to a running console, and nothing else.
//!
//! What the machine then *does* lives in [`console_loop`]; the hardware gates
//! used to debug the M1 interrupt path live in [`selftest`], behind the
//! `bringup` feature. Keeping the three apart matters because only this file
//! has to be read in order: every line here depends on the ones above it.

mod console_loop;
mod console_server;
#[cfg(feature = "oracle")]
mod demos;
mod loader;
#[cfg(feature = "bringup")]
mod selftest;

#[cfg(feature = "oracle")]
use crate::agent::{self};
use crate::arch::{bootinfo, cpu, exception, mmu, timer};
use kernel_core::layout::Region;
use kernel_core::paging::{MemKind, Perms};

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
/// The optional features this image was compiled with, as one word per feature.
///
/// `cfg!` rather than `#[cfg]` so every arm is type-checked in every build: a
/// feature renamed in `Cargo.toml` fails here instead of silently dropping out
/// of the banner, which is the failure mode a `#[cfg]` chain has.
const fn build_features() -> &'static str {
    match (cfg!(feature = "debug-display"), cfg!(feature = "bringup")) {
        (true, true) => "debug-display bringup",
        (true, false) => "debug-display",
        (false, true) => "bringup",
        (false, false) => "headless (no SPI TFT, no bring-up gates)",
    }
}

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
    // The image says what it was built as, on the wire, before anything else
    // can go wrong. A flashed card is otherwise indistinguishable from another
    // one: `kernel8.img` is headless or glass-enabled depending on a `make`
    // invocation nobody can read afterwards, and the symptom of the wrong image
    // is a panel that stays dark — which looks exactly like broken hardware.
    // Cost: one line. What it replaces: guessing.
    println!(uart, "build: {}", build_features());

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

    // Why the board came up, read before anything else can obscure it.
    //
    // `halt()` is `loop { wfe }` with IRQs masked and cannot exit, so a board
    // that boots again after `*** halt ***` was reset from outside this kernel.
    // That was observed twice in one hardware session with no way to tell a
    // firmware watchdog from a brownout. The silicon latches the answer; this
    // line puts it in every transcript, so the question is a lookup rather
    // than an investigation the next time it happens.
    //
    // SAFETY: single core, PM window inside the mapped peripheral region, and
    // this is the only code in the tree that touches the block.
    let reset = unsafe { board::pm::reset_status() };
    println!(
        uart,
        "reset: {:?} partition={} (PM_RSTS={:#010x})", reset.cause, reset.partition, reset.raw
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
        // The scheduler before the first EL0 entry: since ADR-0017 §1 the EL0
        // session lives in the TCB, so *something* has to be the current task
        // before anyone can enter EL0. `init` claims idle — which is the task
        // this bootstrap is already running on — and publishes its session.
        // It used to run after the demos, which was only tenable while the
        // session was a machine-wide global that existed from link time.
        crate::sched::init();

        // M5 S2/S3: AS create → prepare (kernel clone + user stack) → EL0 probes.
        #[cfg(feature = "oracle")]
        demos::m5_aspace_and_el0_smoke(&mut uart);
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
    //
    // The count is printed because a boot that registered nothing looks exactly
    // like a healthy one until the first interrupt that nobody answers — and by
    // then the evidence is a counter rather than a line at the point of failure.
    irq::seal();
    println!(
        uart,
        "irq: sealed with {} handlers registered",
        irq::registered()
    );

    // Arm PL011 RX IRQ into the console ring (GIC line already enabled).
    if interrupts_bound {
        // SAFETY: `uart` is the live console handle this function acquired and
        // still exclusively owns, and EL1 IRQs are masked until the unmask
        // below — so the handler cannot run against a base that is only half
        // published. Guarded on `interrupts_bound` because arming RX into a
        // dispatch table that never bound the line would leave the FIFO to fill
        // with nobody draining it.
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

    // Console channel (M8): send end is what agents (and the loader's `held`
    // list) receive; recv end is held by the resident EL1 console server.
    // Authority is ordinary `CapRights::SEND` — there is no special console
    // capability type after SYS_PUTC's removal.
    let console_cap = match crate::ipc::create_channel() {
        Ok(ch) => {
            match crate::sched::spawn_with_caps(console_server::run, &[ch.recv]) {
                Ok(_) => crate::kprintln!("console-server: up"),
                Err(e) => crate::kprintln!("console-server: spawn FAILED {e:?}"),
            }
            crate::kprintln!("console: capability minted");
            Some(ch.send)
        }
        Err(e) => {
            crate::kprintln!("console: capability FAILED {e:?}");
            None
        }
    };

    // Agents that are data (ADR-0021). One loop over a table, and the table is
    // the *only* place those grants are written — so `held` here is the whole
    // of what any manifest agent can be given, and an entry naming anything
    // else is refused by arithmetic rather than by a check.
    //
    // Product path carries the beacon agent; oracle adds mute. See loader.
    let one;
    let held: &[kernel_core::cap::CapId] = match console_cap {
        Some(cap) => {
            one = [cap];
            &one
        }
        None => &[],
    };
    loader::load_all(held);

    // Everything the boot oracle needs, and nothing the product does. Rule 9
    // of `architecture.md` keeps diagnostic scaffolding out of the production
    // surface; `make product-builds` compiles the image without it and refuses
    // an ELF that still carries a demo symbol.
    #[cfg(feature = "oracle")]
    {
        match crate::sched::spawn(demos::demo_task_a) {
            Ok(_) => crate::kprintln!("sched: spawned task-a"),
            Err(e) => crate::kprintln!("sched: spawn task-a FAILED {e:?}"),
        }
        match crate::sched::spawn(demos::demo_task_b) {
            Ok(_) => crate::kprintln!("sched: spawned task-b"),
            Err(e) => crate::kprintln!("sched: spawn task-b FAILED {e:?}"),
        }

        // Slot 0 is left empty for these two on purpose: their programs name
        // `CONSOLE_SLOT` (1), so the table has a hole under it, and an agent that
        // miscounts its own slots is refused rather than served something adjacent.
        let console_caps: [Option<kernel_core::cap::CapId>; 2] = [None, console_cap];
        // M5-P1/P2: EL0 from a scheduled task + SVC decode.
        match crate::sched::spawn_with_slots(demos::el0_scheduled_task, &console_caps) {
            Ok(_) => crate::kprintln!("sched: spawned el0-task"),
            Err(e) => crate::kprintln!("sched: spawn el0-task FAILED {e:?}"),
        }

        // M6 v1: PL011 page-only agent (ADR-0013); destroy = kill.
        match crate::sched::spawn_with_slots(demos::pl011_agent_task, &console_caps) {
            Ok(_) => crate::kprintln!("sched: spawned pl011-agent"),
            Err(e) => crate::kprintln!("sched: spawn pl011-agent FAILED {e:?}"),
        }

        // K9 / ADR-0034: second peripheral agent (RNG200 page map + kill).
        match crate::sched::spawn(demos::rng_agent_task) {
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
                demos::IPC_FORGE_RAW.store(ch.send.raw(), core::sync::atomic::Ordering::Relaxed);
                match crate::sched::spawn_with_caps(demos::ipc_receiver, &[ch.recv]) {
                    Ok(_) => crate::kprintln!("ipc: spawned receiver"),
                    Err(e) => crate::kprintln!("ipc: spawn receiver FAILED {e:?}"),
                }
                match crate::sched::spawn_with_caps(demos::ipc_sender, &[ch.send]) {
                    Ok(_) => crate::kprintln!("ipc: spawned sender"),
                    Err(e) => crate::kprintln!("ipc: spawn sender FAILED {e:?}"),
                }
                match crate::sched::spawn(demos::ipc_forger) {
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
                match crate::sched::spawn_with_slots(
                    demos::el0_ipc_receiver,
                    &[Some(ch.recv), console_cap],
                ) {
                    Ok(_) => crate::kprintln!("el0-ipc: spawned receiver"),
                    Err(e) => crate::kprintln!("el0-ipc: spawn receiver FAILED {e:?}"),
                }
                match crate::sched::spawn_with_caps(demos::el0_ipc_sender, &[ch.send]) {
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
        match crate::sched::spawn_with_slots(demos::irq_wait_task, &[irq_timer_cap]) {
            Ok(_) => crate::kprintln!("sched: spawned irq-wait"),
            Err(e) => crate::kprintln!("sched: spawn irq-wait FAILED {e:?}"),
        }
        // Empty-slot SYS_WAIT_IRQ must refuse on the good path.
        match crate::sched::spawn(demos::el0_irq_refuse_task) {
            Ok(_) => crate::kprintln!("sched: spawned el0-irq-refuse"),
            Err(e) => crate::kprintln!("sched: spawn el0-irq-refuse FAILED {e:?}"),
        }

        // ADR-0032 / K3: a task that holds SEND revokes the channel; bootstrap
        // then proves the stale CapId refuses send (product path, not forged).
        match crate::ipc::create_channel() {
            Ok(ch) => {
                demos::REVOKE_STALE.store(ch.send.raw(), core::sync::atomic::Ordering::Relaxed);
                match crate::sched::spawn_with_caps(demos::revoke_held_task, &[ch.send]) {
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
                match crate::sched::spawn_with_caps(demos::orphan_receiver, &[ch.recv]) {
                    Ok(id) => {
                        demos::ORPHAN_TASK.store(id.0, core::sync::atomic::Ordering::Relaxed);
                        crate::kprintln!("ipc: orphan spawned id={}", id.0);
                    }
                    Err(e) => crate::kprintln!("ipc: orphan spawn FAILED {e:?}"),
                }
                // `ch.send` dropped here: nobody holds send.
                match crate::sched::spawn(demos::orphan_reaper) {
                    Ok(_) => crate::kprintln!("ipc: reaper spawned"),
                    Err(e) => crate::kprintln!("ipc: reaper spawn FAILED {e:?}"),
                }
            }
            Err(e) => crate::kprintln!("ipc: orphan channel FAILED {e:?}"),
        }

        // ADR-0033 / K10: product supervisor reaps a blocked child and restarts.
        match crate::sched::spawn(demos::supervisor_task) {
            Ok(_) => crate::kprintln!("sched: spawned supervisor"),
            Err(e) => crate::kprintln!("sched: spawn supervisor FAILED {e:?}"),
        }

        // ADR-0031 / K2: ephemeral channel — sole SEND holder exits, waiter
        // is auto-cancelled without a supervisor reaper.
        match crate::ipc::create_channel_ephemeral() {
            Ok(ch) => {
                match crate::sched::spawn_with_caps(demos::auto_reap_receiver, &[ch.recv]) {
                    Ok(_) => crate::kprintln!("ipc: auto-reap receiver spawned"),
                    Err(e) => crate::kprintln!("ipc: auto-reap receiver FAILED {e:?}"),
                }
                match crate::sched::spawn_with_caps(demos::auto_reap_sender, &[ch.send]) {
                    Ok(_) => crate::kprintln!("ipc: auto-reap sender spawned"),
                    Err(e) => crate::kprintln!("ipc: auto-reap sender FAILED {e:?}"),
                }
            }
            Err(e) => crate::kprintln!("ipc: auto-reap channel FAILED {e:?}"),
        }
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
