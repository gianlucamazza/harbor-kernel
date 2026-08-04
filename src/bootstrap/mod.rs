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
/// One per task that lands in a 2 MiB heap block not yet split, plus the device
/// tree's, plus margin. `MAX_TASKS` is 4, so three workers can each cost one.
const MIN_SPARE_TABLES: usize = 6;

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

    // The heap bound is decided before the map is built, because the heap is
    // one of the regions being mapped.
    let heap_end = (mm::heap_start() + HEAP_SIZE).min(board::memmap::IDENTITY_RAM_END);
    let mut region_buffer = [mm::layout::empty_region(); mm::layout::MAX_REGIONS];
    let regions = match mm::layout::kernel_regions(heap_end as u64, &mut region_buffer) {
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

    // SoC RNG200 soft probe: after MMU (Device attributes), before IRQs.
    // Gated: QEMU raspi4b does not map `0xFE10_4000` (external abort → panic).
    // On silicon: `--features hw-rng`. Logical failure is soft — one line, continue.
    #[cfg(feature = "hw-rng")]
    {
        // SAFETY: single core; RNG200 window not otherwise claimed.
        match unsafe { board::rng::init() } {
            Ok(rng) => match rng.try_word() {
                Ok(Some(word)) => println!(uart, "rng200: ok word={word:#010x}"),
                Ok(None) => {
                    let mut words = [0u32; 1];
                    match rng.read_words(&mut words) {
                        Ok(1) => println!(uart, "rng200: ok word={:#010x}", words[0]),
                        Ok(_) => println!(uart, "rng200: ok (FIFO empty after warm-up)"),
                        Err(error) => {
                            println!(uart, "rng200: warm-up ok, read FAILED: {error:?}")
                        }
                    }
                }
                Err(error) => println!(uart, "rng200: read FAILED: {error:?}"),
            },
            Err(error) => println!(uart, "rng200: unavailable ({error:?})"),
        }
    }

    // Optional SPI0 stack for the status TFT (ADR-0009). After MMU so SPI MMIO
    // is Device-mapped; before IRQs so a wedged controller cannot interrupt.
    // The handle is dropped after the diagnostic line until the panel driver
    // owns it for the status surface — construction alone proves pinmux + CDIV.
    #[cfg(feature = "debug-display")]
    {
        board::display::smoke_delays();
        // SAFETY: single core; GPIO/SPI0 not otherwise claimed.
        match unsafe { board::display::init_spi() } {
            Ok(spi) => {
                // Keep handles live for the diagnostic line; panel attach is next.
                let _ = (spi.device, spi.dc, spi.rst);
                println!(
                    uart,
                    "SPI0 ready  cdiv={}  bit_clk={} Hz (debug-display)", spi.cdiv, spi.bit_hz
                );
            }
            Err(error) => println!(uart, "SPI0 init FAILED: {error:?}"),
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
