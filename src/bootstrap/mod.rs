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
mod discover;
mod loader;
#[cfg(feature = "bringup")]
mod selftest;

#[cfg(feature = "oracle")]
use crate::agent::{self};
use crate::arch::{bootinfo, cpu, exception, mmu, smp, timer};
use kernel_core::asid::ASID_BITS;
use kernel_core::cpuid;
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
/// The PA-range field on the `cpu:` line: `pa44`, or `pa?` when the encoding
/// is one the architecture reserves. Diagnostic only, so a reserved value must
/// not refuse the boot — but it must not print as a plausible number either.
struct PaBits(Option<u32>);

impl core::fmt::Display for PaBits {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(bits) => write!(f, "{bits}"),
            None => f.write_str("?"),
        }
    }
}

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
    // `src=` is `git describe --always --dirty` at compile time (build.rs).
    // The feature word says what the image *does*; the source id says what it
    // was built *from*, which is what a transcript cited by an ADR needs in
    // order to be evidence about a specific tree rather than about "a build".
    println!(
        uart,
        "build: {} src={} {}",
        build_features(),
        env!("HARBOR_SOURCE_ID"),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
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
    // The map is live: remember the two bounds it was built from, so a later
    // fault can be told which region its address belongs to.
    mm::layout::record_bounds(heap_end as u64, frame_end as u64);

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
    // Phase marks for the `boot:` timing line at the end of bring-up. Sampled
    // here rather than derived from the console timestamps, which belong to
    // the host and only exist when someone was capturing.
    let mmu_at = timer::physical_count();

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

    // Which core this is, checked against the core the kernel was built for
    // (ADR-0065). The A72 knowledge in this tree — pre-MMU exclusives
    // confinement, explicit I/D-cache maintenance, ADR-0050's ASID arithmetic
    // — was comment-only until this line put the observed identity in every
    // transcript. Load-bearing mismatches refuse the boot; an unknown part is
    // a distinct printed outcome and the boot continues, because the
    // A72-specific handling is conservative on other cores and what the check
    // owes there is visibility, not a verdict.
    let midr = cpu::midr_el1();
    let mmfr0 = cpu::id_aa64mmfr0_el1();
    let pfr0 = cpu::id_aa64pfr0_el1();
    if !cpuid::tgran4_supported(mmfr0) || !cpuid::el0_aarch64(pfr0) || !cpuid::el1_aarch64(pfr0) {
        // The whole paging model is written against the 4 KiB granule, and the
        // session model against AArch64 EL0/EL1. There is nothing to degrade to.
        refuse_to_boot(
            &mut uart,
            format_args!(
                "core lacks the 4 KiB granule or AArch64 EL0/EL1 \
                 (ID_AA64MMFR0={mmfr0:#x}, ID_AA64PFR0={pfr0:#x})"
            ),
        )
    }
    let asid = match cpuid::asid_bits(mmfr0) {
        Some(bits) if bits >= ASID_BITS => bits,
        // Fewer hardware bits than the pool hands out would alias two address
        // spaces in the TLB — ADR-0050's isolation, gone silently. A reserved
        // encoding refuses too: a width nobody can name cannot back the pool.
        answer => refuse_to_boot(
            &mut uart,
            format_args!(
                "hardware ASID width {answer:?} cannot back the {ASID_BITS}-bit \
                 pool of ADR-0050 (ID_AA64MMFR0={mmfr0:#x})"
            ),
        ),
    };
    let pa = PaBits(cpuid::pa_bits(mmfr0));
    match cpuid::part(midr) {
        cpuid::Part::CortexA72 => println!(
            uart,
            "cpu: Cortex-A72 r{}p{} asid{asid} pa{pa} (MIDR={:#010x})",
            cpuid::variant(midr),
            cpuid::revision(midr),
            midr & 0xFFFF_FFFF
        ),
        cpuid::Part::Unknown { implementer, part } => println!(
            uart,
            "cpu: unknown implementer={implementer:#04x} part={part:#05x} r{}p{} \
             asid{asid} pa{pa} (MIDR={:#010x})",
            cpuid::variant(midr),
            cpuid::revision(midr),
            midr & 0xFFFF_FFFF
        ),
    }

    // ADR-0070 / K8 first slice: unpark core 1 into an idle loop. Kernel map
    // and VBAR are live; IRQs still masked. Timeout prints an honest line the
    // boot oracle fails on — silence would look like a single-core boot.
    let seen = smp::secondary_seen_count();
    let core1 = smp::unpark_core1();
    if core1 {
        println!(uart, "smp: core1 alive");
    } else {
        println!(uart, "smp: core1 timeout seen={seen}");
    }

    // The kernel map deliberately covers far less than the early one, so the
    // device-tree blob is now outside it. Map it back in: it is the first
    // region whose address only the firmware knows, which is exactly what
    // `mmu::map` exists for.
    let mut dtb_mapped = false;
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
            Ok(()) => {
                println!(uart, "DTB mapped: {len} bytes at {base:#x}");
                dtb_mapped = true;
            }
            Err(error) => println!(uart, "DTB map FAILED: {error:?}"),
        }
    }

    // The discovery report (ADR-0072/0073): observe the tree, reconcile with
    // the compiled claims, print one line per fact. Fail-open — a missing or
    // unmappable blob prints its `unknown` form and the boot continues.
    // smp-seen: primary always + core1 if unpark returned alive (not the
    // secondary_wait counter — that stays 0 under QEMU -kernel).
    let smp_seen = 1u64 + u64::from(core1);
    discover::report(&mut uart, dtb_mapped, smp_seen);
    let discover_at = timer::physical_count();

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

    // ADR-0074 / K8 second slice: core 1 banked GICC + SGI 0 wake. Needs the
    // shared table sealed (handler present) and the distributor open. Runs
    // before primary unmask — the *target* unmasks inside secondary idle.
    if core1 && interrupts_bound {
        if board::irq::probe_core1_ipi() {
            println!(uart, "smp: core1 ipi");
        } else {
            println!(uart, "smp: core1 ipi timeout");
        }
        // ADR-0076: multi-current — reserve CPU1 idle, pin a marker worker,
        // wait for it to run (primary prints — no console TX from core 1).
        match crate::sched::start_cpu1() {
            Ok(_) => {
                match crate::sched::spawn_core1_marker() {
                    Ok(_) => {
                        if crate::sched::wait_core1_ran(200_000_000) {
                            println!(uart, "smp: core1 ran");
                        } else {
                            println!(uart, "smp: core1 ran timeout");
                        }
                    }
                    Err(e) => println!(uart, "smp: core1 spawn FAILED {e:?}"),
                }
            }
            Err(e) => println!(uart, "smp: core1 start FAILED {e:?}"),
        }
    } else if core1 {
        println!(uart, "smp: core1 ipi skipped (irq unbound)");
    }

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

    // Shared TX for idle + worker tasks (serialized in with_tx; not a claim
    // that the whole kernel is cooperative-only — see K4 preemption).
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
                match crate::sched::spawn(demos::transfer_recipient_task) {
                    Ok(to) => {
                        demos::TRANSFER_TO
                            .store(to.to_raw(), core::sync::atomic::Ordering::Relaxed);
                        match crate::sched::spawn_with_caps(demos::transfer_donor_task, &[ch.send])
                        {
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
        match crate::sched::spawn(demos::cascade_parent_task) {
            Ok(_) => crate::kprintln!("cascade: parent spawned"),
            Err(e) => crate::kprintln!("cascade: parent spawn FAILED {e:?}"),
        }

        // ADR-0039 / P5 residual: bind short name, EL0 resolves into empty slot.
        match crate::ipc::create_channel() {
            Ok(ch) => {
                let _ = crate::naming::bind(b"ab", ch.send);
                match crate::sched::spawn(demos::el0_resolve_task) {
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
                match crate::sched::spawn_with_caps(demos::timeout_recv_task, &[ch.recv]) {
                    Ok(_) => crate::kprintln!("ipc: timeout-recv spawned"),
                    Err(e) => crate::kprintln!("ipc: timeout-recv spawn FAILED {e:?}"),
                }
                // Keep send off-task so the waiter is not auto-reaped by K2 hold drop.
                let _ = ch.send;
            }
            Err(e) => crate::kprintln!("ipc: timeout channel FAILED {e:?}"),
        }

        // ADR-0041 / K3 residual: EL0 transfer return-to-creator.
        match crate::sched::spawn(demos::el0_transfer_parent_task) {
            Ok(_) => crate::kprintln!("el0-xfer: parent spawned"),
            Err(e) => crate::kprintln!("el0-xfer: parent spawn FAILED {e:?}"),
        }
        match crate::sched::spawn(demos::el0_transfer_refuse_task) {
            Ok(_) => crate::kprintln!("el0-xfer: refuse spawned"),
            Err(e) => crate::kprintln!("el0-xfer: refuse spawn FAILED {e:?}"),
        }

        // ADR-0054 / K3 residual: EL0 peer transfer via task-cap.
        match crate::sched::spawn(demos::el0_peer_xfer_parent_task) {
            Ok(_) => crate::kprintln!("el0-xfer-peer: parent spawned"),
            Err(e) => crate::kprintln!("el0-xfer-peer: parent spawn FAILED {e:?}"),
        }
        match crate::sched::spawn(demos::el0_peer_xfer_refuse_task) {
            Ok(_) => crate::kprintln!("el0-xfer-peer: refuse spawned"),
            Err(e) => crate::kprintln!("el0-xfer-peer: refuse spawn FAILED {e:?}"),
        }

        // ADR-0055 / ADR-0057: band filter + stale task-cap refusal.
        match crate::sched::spawn(demos::xfer_peer_stale_task) {
            Ok(_) => crate::kprintln!("xfer-peer: stale spawned"),
            Err(e) => crate::kprintln!("xfer-peer: stale spawn FAILED {e:?}"),
        }

        // ADR-0042 / K2 residual: EL0 SYS_RECV_TIMEOUT.
        match crate::ipc::create_channel() {
            Ok(ch) => {
                match crate::sched::spawn_with_caps(demos::el0_timeout_task, &[ch.recv]) {
                    Ok(_) => crate::kprintln!("el0-timeout: spawned"),
                    Err(e) => crate::kprintln!("el0-timeout: spawn FAILED {e:?}"),
                }
                let _ = ch.send;
            }
            Err(e) => crate::kprintln!("el0-timeout: channel FAILED {e:?}"),
        }

        // ADR-0043 / K9 residual: IRQ-cap device wait is proven sequentially
        // inside irq_wait_task (one waiter per cookie — no concurrent race).

        // ADR-0044 / K5: thin-stack density workers.
        {
            let mut n = 0u32;
            for _ in 0..3 {
                match crate::sched::spawn_thin(demos::density_thin_task) {
                    Ok(_) => n += 1,
                    Err(_) => break,
                }
            }
            let each = kernel_core::density::bytes_per_task(kernel_core::density::StackClass::Thin);
            crate::kprintln!("density: thin n={n} bytes_each={each}");
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
        match crate::sched::spawn_thin(demos::preempt_el1_peer) {
            Ok(_) => match crate::sched::spawn_thin(demos::preempt_el1_spinner) {
                Ok(_) => crate::kprintln!("preempt-el1: workers spawned"),
                Err(e) => crate::kprintln!("preempt-el1: spinner spawn FAILED {e:?}"),
            },
            Err(e) => crate::kprintln!("preempt-el1: peer spawn FAILED {e:?}"),
        }

        // ADR-0079 / K8: same claim on home=1 — local CNTP + EL1 epilogue.
        // Watcher on CPU0 prints (no TX from core 1); peer then spinner on 1.
        match crate::sched::spawn_thin(demos::preempt_el1_cpu1_watch) {
            Ok(_) => match crate::sched::spawn_on(1, demos::preempt_el1_cpu1_peer) {
                Ok(_) => match crate::sched::spawn_on(1, demos::preempt_el1_cpu1_spinner) {
                    Ok(_) => crate::kprintln!("preempt-el1-cpu1: workers spawned"),
                    Err(e) => crate::kprintln!("preempt-el1-cpu1: spinner spawn FAILED {e:?}"),
                },
                Err(e) => crate::kprintln!("preempt-el1-cpu1: peer spawn FAILED {e:?}"),
            },
            Err(e) => crate::kprintln!("preempt-el1-cpu1: watch spawn FAILED {e:?}"),
        }

        // ADR-0081 / K8: EL0 session + quantum preemption on home=1.
        // Publish is per-CPU; peer then spinner on 1; watcher prints on 0.
        match crate::sched::spawn_thin(demos::el0_cpu1_watch) {
            Ok(_) => match crate::sched::spawn_on(1, demos::el0_cpu1_peer) {
                Ok(_) => match crate::sched::spawn_on(1, demos::el0_cpu1_spinner) {
                    Ok(_) => crate::kprintln!("preempt-el0-cpu1: workers spawned"),
                    Err(e) => crate::kprintln!("preempt-el0-cpu1: spinner spawn FAILED {e:?}"),
                },
                Err(e) => crate::kprintln!("preempt-el0-cpu1: peer spawn FAILED {e:?}"),
            },
            Err(e) => crate::kprintln!("preempt-el0-cpu1: watch spawn FAILED {e:?}"),
        }

        // ADR-0083 / K8: work steal — all admitted on CPU0; no spawn_on(1).
        // Two cooperative victims so one is Ready while the other runs; CPU1 pulls.
        match crate::sched::spawn_thin(demos::steal_watch) {
            Ok(_) => match crate::sched::spawn_thin(demos::steal_victim) {
                Ok(_) => match crate::sched::spawn_thin(demos::steal_victim) {
                    Ok(_) => crate::kprintln!("smp: steal workers spawned"),
                    Err(e) => crate::kprintln!("smp: steal victim2 spawn FAILED {e:?}"),
                },
                Err(e) => crate::kprintln!("smp: steal victim spawn FAILED {e:?}"),
            },
            Err(e) => crate::kprintln!("smp: steal watch spawn FAILED {e:?}"),
        }

        // ADR-0064 / K4: IRQ-side preemption of a non-syscalling EL0 spinner.
        // Peer first, so it is already in the rotation when the window opens.
        match crate::sched::spawn_thin(demos::preempt_peer_task) {
            Ok(_) => match crate::sched::spawn(demos::preempt_agent_task) {
                Ok(_) => crate::kprintln!("preempt: tasks spawned"),
                Err(e) => crate::kprintln!("preempt: agent spawn FAILED {e:?}"),
            },
            Err(e) => crate::kprintln!("preempt: peer spawn FAILED {e:?}"),
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
                        demos::ORPHAN_TASK
                            .store(id.to_raw(), core::sync::atomic::Ordering::Relaxed);
                        crate::kprintln!("ipc: orphan spawned id={}", id.slot());
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

    // How long the phases above took, on the board's own clock. The serial
    // transcript carries host timestamps, but only when someone captured it
    // with `serial-capture`; a log read on its own could not answer "is this
    // slow?" at all. `CNTPCT` runs from reset, so these are milliseconds since
    // the counter started, not since power — which is what a comparison
    // between two boots of the same image needs.
    let hz = timer::frequency_hz();
    crate::kprintln!(
        "boot: mmu={} ms discover={} ms ready={} ms",
        kernel_core::delay::counts_to_ms(hz, mmu_at),
        kernel_core::delay::counts_to_ms(hz, discover_at),
        kernel_core::delay::counts_to_ms(hz, timer::physical_count())
    );

    // Idle body — never returns (ADR-0006).
    console_loop::run()
}
