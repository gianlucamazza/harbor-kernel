//! Kernel bring-up: the ordered sequence that takes the machine from `_start`
//! to a running console, and nothing else.
//!
//! What the machine then *does* lives in [`shell`]; the hardware gates used to
//! debug the M1 interrupt path live in [`selftest`], behind the `bringup`
//! feature. Keeping the three apart matters because only this file has to be
//! read in order: every line here depends on the ones above it.

#[cfg(feature = "bringup")]
mod selftest;
mod shell;

use crate::arch::{bootinfo, cpu, exception, mmu, timer};
use crate::bsp::board;
use crate::console;
use crate::irq;
use crate::mm;
use crate::println;

/// Timer rate (IRQ ticks per second).
const TIMER_HZ: u32 = 10;

/// Kernel heap size, clamped to the identity-mapped RAM window.
const HEAP_SIZE: usize = 64 * 1024 * 1024;

/// Full kernel bring-up; never returns.
pub fn run() -> ! {
    // SAFETY: core 0; DAIF still masked from `boot.s`; nothing else has run.
    let Some(mut uart) = (unsafe { console::acquire() }) else {
        // Unreachable in practice — this is the first claim — but there is no
        // console to report it on, so park rather than pretend.
        cpu::halt()
    };

    println!(uart, "rpi_minimal_agentic: hello");
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
    let regions = mm::layout::kernel_regions(heap_end as u64);

    // Swap the coarse early map for the real one. On failure the early map
    // stays active, so the report below still reaches the console.
    // SAFETY: single core, IRQs masked, early map active.
    match unsafe { mmu::activate(&regions) } {
        Ok(()) => {
            println!(
                uart,
                "MMU {}  (W^X, guard page at {:#x}, {} B of table arena left)",
                if mmu::is_enabled() { "on" } else { "OFF?" },
                mm::layout::guard_page(),
                mmu::tables_remaining()
            );
        }
        Err((error, region)) => {
            println!(uart, "MMU FAILED mapping {region}: {error:?}");
            println!(uart, "continuing unmapped — addresses are physical");
        }
    }

    // SAFETY: the heap region was just mapped as Normal memory.
    let heap_ok = unsafe { mm::init_heap(heap_end) };
    if heap_ok {
        println!(uart, "heap remaining = {} bytes", mm::heap_remaining());
    } else {
        println!(uart, "heap UNAVAILABLE (empty region)");
    }

    // SAFETY: single-core; exclusive GIC ownership.
    if let Err(error) = unsafe { board::irq::init(TIMER_HZ) } {
        println!(uart, "IRQ bind FAILED: {error:?}");
        println!(uart, "no timer or RX interrupts — console is output only");
    }
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
    unsafe {
        console::enable_rx_irq(&uart);
    }

    // Clean periodic deadline, then unmask IRQs.
    timer::on_interrupt();
    cpu::sync_pipeline();
    cpu::irq_enable();
    cpu::sync_pipeline();
    println!(uart, "IRQs enabled (timer + UART RX)");
    println!(uart, "idle: WFI when no RX/tick work");

    shell::heap_check(&mut uart);

    shell::run(&mut uart)
}
