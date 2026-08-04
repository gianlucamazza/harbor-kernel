//! Kernel bring-up.
//!
//! Production path: vectors → irqchip+timer → unmask → console.
//! Optional self-test (masked soft proof + HPPIR + IAR) via [`BRINGUP_SELFTEST`].

use crate::arch::{cpu, exception, mmu, timer};
use crate::bsp::board;
use crate::console;
use crate::drivers::pl011::Pl011;
use crate::irq;
use crate::mm;
use crate::println;
use crate::time;

/// Timer rate (IRQ ticks per second).
const TIMER_HZ: u32 = 10;

/// Kernel heap size, clamped to the identity-mapped RAM window.
const HEAP_SIZE: usize = 64 * 1024 * 1024;

/// Print `ticks=` every this many IRQ ticks (~1 Hz at [`TIMER_HZ`] = 10).
const TICK_PRINT_EVERY: u64 = 10;

/// When `true`, run the full masked self-test used to debug M1 GIC/timer.
/// Leave `false` for normal boots (HW path already validated on Pi 4B Rev 1.5).
const BRINGUP_SELFTEST: bool = false;

const SOFT_PROOF_COUNT: u32 = 3;

/// Full kernel bring-up; never returns.
pub fn run() -> ! {
    // SAFETY: core 0; DAIF still masked from `boot.s`.
    let mut uart = unsafe { console::acquire() };

    println!(uart, "rpi_minimal_agentic: hello");
    println!(uart, "M2+P0: MMU + heap + idle + UART RX IRQ");

    exception::init();

    // No `CPACR_EL1.FPEN` here on purpose: the kernel is built for
    // `aarch64-unknown-none-softfloat`, so it contains no FP/SIMD at all
    // (enforced by `make no-simd`). Leaving FPEN clear means any future stray
    // FP instruction traps loudly instead of silently corrupting the IRQ path,
    // whose trap frame saves no q registers.

    // The heap bound is decided before the map is built, because the heap is
    // one of the regions being mapped.
    let heap_end = (mm::heap_start() + HEAP_SIZE).min(board::memmap::IDENTITY_RAM_END);
    let regions = mm::layout::kernel_regions(heap_end as u64);

    // SAFETY: build the map before enabling translation; IRQs still masked.
    match unsafe { mmu::enable(&regions) } {
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
    let irq_bound = unsafe { board::irq::init(TIMER_HZ) };
    if !irq_bound {
        println!(uart, "IRQ bind FAILED: a handler id is out of range");
    }
    cpu::sync_pipeline();

    println!(
        uart,
        "CNTFRQ={} Hz  timer={} Hz  PPI={}",
        timer::frequency_hz(),
        TIMER_HZ,
        board::irq::TIMER_IRQ
    );

    if BRINGUP_SELFTEST && !selftest(&mut uart) {
        println!(uart, "SELFTEST FAIL — soft console (IRQs masked)");
        soft_console(&mut uart);
    }

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

    heap_check(&mut uart);

    irq_console(&mut uart)
}

/// Exercise the global allocator and report whether memory comes back.
///
/// The interesting number is the one after the drop: a bump allocator prints a
/// smaller figure there, a real one prints the figure it started with.
fn heap_check(uart: &mut Pl011) {
    use alloc::boxed::Box;
    use alloc::vec::Vec;

    let before = mm::heap_remaining();

    {
        let boxed = Box::new(0xA5A5_A5A5u32);
        let mut grown: Vec<u64> = Vec::new();
        for i in 0..1024 {
            grown.push(i);
        }
        let sum: u64 = grown.iter().sum();

        println!(
            uart,
            "heap: Box at {:p}, Vec of {} sums to {sum}",
            &*boxed,
            grown.len()
        );
        println!(
            uart,
            "heap: {} bytes free while held, {} fragments",
            mm::heap_remaining(),
            mm::heap_fragments()
        );
    }

    let after = mm::heap_remaining();
    println!(
        uart,
        "heap: {after} bytes free after drop ({}), {} fragments",
        if after == before {
            "fully reclaimed"
        } else {
            "LEAKED"
        },
        mm::heap_fragments()
    );
}

// ---------------------------------------------------------------------------
// Self-test (optional)
// ---------------------------------------------------------------------------

fn selftest(uart: &mut Pl011) -> bool {
    println!(uart, "selftest: soft CNTP…");
    let soft = soft_proof(uart);
    println!(uart, "selftest: soft_ticks={soft}");
    if soft == 0 {
        println!(uart, "selftest: FAIL CNTP");
        return false;
    }

    println!(uart, "selftest: HPPIR gate…");
    if !gic_sees_timer_pending(uart) {
        println!(uart, "selftest: FAIL HPPIR");
        return false;
    }

    println!(uart, "selftest: IAR claim…");
    if !software_inject_timer(uart) {
        println!(uart, "selftest: FAIL IAR");
        return false;
    }

    println!(uart, "selftest: OK");
    true
}

fn soft_proof(uart: &mut Pl011) -> u32 {
    let mut soft = 0u32;
    for _ in 0..80_000_000u32 {
        if timer::is_pending() {
            timer::on_interrupt();
            soft += 1;
            println!(uart, "soft: fire #{soft}");
            if soft >= SOFT_PROOF_COUNT {
                break;
            }
        }
    }
    soft
}

fn gic_sees_timer_pending(uart: &mut Pl011) -> bool {
    let counts = (timer::frequency_hz() / 1000).max(1000);
    timer::set_deadline_counts(counts);
    cpu::sync_pipeline();

    for _ in 0..20_000_000u32 {
        if timer::is_pending() {
            if let Some(id) = irq::peek_pending() {
                if id == board::irq::TIMER_IRQ {
                    println!(uart, "gate: HPPIR={id} ok");
                    timer::on_interrupt();
                    return true;
                }
            }
        }
    }
    println!(
        uart,
        "gate: timeout timer={} hppir={:?}",
        timer::is_pending() as u8,
        irq::peek_pending()
    );
    timer::on_interrupt();
    false
}

fn software_inject_timer(uart: &mut Pl011) -> bool {
    let before = time::ticks();

    // Avoid CNTP reprogram while the line is live on the GIC.
    irq::disable(board::irq::TIMER_IRQ);
    cpu::sync_pipeline();
    timer::on_interrupt();
    time::tick();

    irq::enable(board::irq::TIMER_IRQ);
    let counts = (timer::frequency_hz() / 1000).max(1000);
    timer::set_deadline_counts(counts);
    cpu::sync_pipeline();

    for _ in 0..20_000_000u32 {
        if timer::is_pending() && irq::peek_pending() == Some(board::irq::TIMER_IRQ) {
            break;
        }
    }

    let iar = board::irq::debug_read_iar();
    let id = iar & 0x3FF;
    println!(uart, "inject: IAR={iar:#x} id={id}");

    if id == board::irq::TIMER_IRQ {
        timer::on_interrupt();
        time::tick();
        board::irq::debug_write_eoir(iar);
    } else if id != 1023 {
        board::irq::debug_write_eoir(iar);
    } else if timer::is_pending() {
        timer::on_interrupt();
    }

    let after = time::ticks();
    println!(uart, "inject: ticks {before} -> {after}");
    after > before
}

fn soft_console(uart: &mut Pl011) -> ! {
    let mut soft: u64 = 0;
    let mut last = 0u64;
    loop {
        if timer::is_pending() {
            timer::on_interrupt();
            soft += 1;
            if soft >= last + TICK_PRINT_EVERY {
                println!(uart, "soft_ticks={soft}");
                last = soft - (soft % TICK_PRINT_EVERY);
            }
        }
        echo_byte(uart);
    }
}

/// Event-driven console: drain RX ring, report ticks, idle with WFI.
fn irq_console(uart: &mut Pl011) -> ! {
    let mut last_printed = 0u64;
    let mut last_counters = irq::Counters::default();
    loop {
        // 1. Echo all bytes the UART RX IRQ pushed into the ring.
        while let Some(b) = console::pop_rx() {
            match b {
                b'\r' => uart.write_bytes(b"\r\n"),
                b => uart.write_byte(b),
            }
        }

        // 2. Periodic tick report (~1 Hz), plus dispatch anomalies. Reporting
        //    the counters only when they move keeps a quiet system quiet while
        //    making an interrupt storm impossible to mistake for idleness.
        let ticks = time::ticks();
        if ticks >= last_printed + TICK_PRINT_EVERY {
            let report = ticks - (ticks % TICK_PRINT_EVERY);
            if report > last_printed {
                println!(uart, "ticks={report}");
                last_printed = report;

                let counters = irq::counters();
                if counters != last_counters {
                    println!(
                        uart,
                        "irq: unhandled={} out_of_range={} loop_exhausted={}",
                        counters.unhandled,
                        counters.out_of_range,
                        counters.loop_exhausted
                    );
                    last_counters = counters;
                }
            }
        }

        // 3. Idle when nothing is pending. Timer (10 Hz) and UART RX both wake.
        //
        //    The check and the sleep run with IRQs masked so an interrupt
        //    arriving between them cannot be lost: `WFI` still completes on a
        //    pending interrupt with `DAIF.I` set, and `without_irqs` restores
        //    the mask so the handler runs immediately after.
        cpu::without_irqs(|| {
            if console::rx_is_empty() && time::ticks() < last_printed + TICK_PRINT_EVERY {
                cpu::wait_for_interrupt();
            }
        });
    }
}

/// Soft-console path (IRQs masked): poll the UART FIFO directly.
fn echo_byte(uart: &mut Pl011) {
    if let Some(b) = uart.read_byte() {
        match b {
            b'\r' => uart.write_bytes(b"\r\n"),
            b => uart.write_byte(b),
        }
    }
}
