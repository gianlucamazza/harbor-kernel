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

    // SAFETY: identity map before enabling translation; IRQs still masked.
    let mmu_result = unsafe {
        mmu::enable_identity(
            &board::memmap::IDENTITY_RAM_BLOCKS,
            &[board::memmap::DEVICE_BLOCK],
        )
    };
    match mmu_result {
        Ok(()) => println!(
            uart,
            "MMU {}  (identity 2GiB RAM + device window)",
            if mmu::is_enabled() { "on" } else { "OFF?" }
        ),
        Err(e) => println!(uart, "MMU FAILED: {e:?}  (continuing unmapped)"),
    }

    // Heap: from linker `__heap_start` for 64 MiB (within identity-mapped RAM).
    // SAFETY: MMU maps this range as Normal memory.
    let heap_ok = unsafe {
        let end = (mm::heap_start() + 64 * 1024 * 1024).min(board::memmap::IDENTITY_RAM_END);
        mm::init_heap(end)
    };
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

    // M2 demo: bump-allocate a small buffer and show its address.
    if let Some(buf) = mm::alloc_zeroed(64, 16) {
        // SAFETY: 64-byte zeroed allocation.
        unsafe {
            core::ptr::write_bytes(buf, b'M', 4);
        }
        println!(uart, "heap demo: alloc 64B at {buf:p}");
    } else {
        println!(uart, "heap demo: alloc FAILED");
    }
    println!(uart, "heap remaining = {} bytes", mm::heap_remaining());

    irq_console(&mut uart)
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
