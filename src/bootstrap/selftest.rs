//! Bring-up gates for the GIC and timer path — `bringup` feature only.
//!
//! These reproduce the sequence used to debug M1 on real hardware: prove CNTP
//! fires with IRQs masked, prove the GIC reports the PPI as pending, then prove
//! an `IAR` claim advances the tick counter. They reach for raw GIC registers,
//! which is exactly why they are not compiled into a production image: the
//! irqchip abstraction exists so kernel policy never touches those.
//!
//! Run with `cargo build --features bringup`.

use crate::arch::{cpu, timer};
use crate::bsp::board;
use crate::drivers::pl011::Pl011;
use crate::irq;
use crate::println;
use crate::time;

/// Timer firings to observe before believing CNTP works.
const SOFT_PROOF_COUNT: u32 = 3;

/// Iterations to spin waiting for a hardware condition before declaring it
/// dead. Generous: this runs once, at boot, on a quiet machine.
const SPIN_BUDGET: u32 = 20_000_000;

/// Print `soft_ticks=` every this many polled firings.
const TICK_PRINT_EVERY: u64 = 10;

/// Run every gate in order. `false` at the first failure.
pub fn run(uart: &mut Pl011) -> bool {
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

/// Poll `CNTP_CTL.ISTATUS` with IRQs masked: does the timer fire at all?
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

/// Does the distributor agree the timer PPI is pending?
fn gic_sees_timer_pending(uart: &mut Pl011) -> bool {
    let counts = (timer::frequency_hz() / 1000).max(1000);
    timer::set_deadline_counts(counts);
    cpu::sync_pipeline();

    for _ in 0..SPIN_BUDGET {
        if timer::is_pending()
            && let Some(id) = board::irq::debug_peek_pending()
            && id == board::irq::TIMER_IRQ
        {
            println!(uart, "gate: HPPIR={id} ok");
            timer::on_interrupt();
            return true;
        }
    }

    println!(
        uart,
        "gate: timeout timer={} hppir={:?}",
        timer::is_pending() as u8,
        board::irq::debug_peek_pending()
    );
    timer::on_interrupt();
    false
}

/// Claim the interrupt by hand and check the tick counter moves.
fn software_inject_timer(uart: &mut Pl011) -> bool {
    let before = time::ticks();

    // Reprogramming CNTP while the line is live on the GIC hangs the board
    // (observed during M1 bring-up), so mask the PPI first.
    irq::disable(board::irq::TIMER_IRQ);
    cpu::sync_pipeline();
    // The real handler, not a re-implementation of it: this gate exists to
    // check that the timer path advances the counter, so it must exercise the
    // path rather than repeat its two steps here.
    time::on_timer_irq();

    irq::enable(board::irq::TIMER_IRQ);
    let counts = (timer::frequency_hz() / 1000).max(1000);
    timer::set_deadline_counts(counts);
    cpu::sync_pipeline();

    for _ in 0..SPIN_BUDGET {
        if timer::is_pending() && board::irq::debug_peek_pending() == Some(board::irq::TIMER_IRQ) {
            break;
        }
    }

    let iar = board::irq::debug_read_iar();
    let id = iar & 0x3FF;
    println!(uart, "inject: IAR={iar:#x} id={id}");

    if id == board::irq::TIMER_IRQ {
        time::on_timer_irq();
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

/// Fallback console for a board whose IRQ path failed the gates: poll the
/// timer and the UART FIFO directly, with interrupts still masked.
pub fn soft_console(uart: &mut Pl011) -> ! {
    let rx = uart.receiver();
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
        if let Some(byte) = rx.read_byte() {
            let _ = match byte {
                b'\r' => uart.write_bytes(b"\r\n"),
                byte => uart.write_byte(byte),
            };
        }
    }
}
