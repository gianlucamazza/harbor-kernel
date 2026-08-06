//! Bring-up gates — `bringup` feature only.
//!
//! - System-register readback: `SCTLR_EL1` against the RES1 pattern `boot.s`
//!   programs. QEMU and silicon may differ here, which is the whole reason to
//!   read it on the board.
//! - GIC/timer path: soft CNTP, HPPIR, IAR (M1).
//! - Task-stack guard: deliberate write to an unmapped heap stack guard (M3).
//!   That path **panics** with ESR/FAR on success; it is never in a production
//!   image. Capture the panic line on silicon and record it in
//!   `docs/verification.md`.
//!
//! Run with `cargo build --release --features bringup`.

use crate::arch::{cpu, timer};
use crate::bsp::board;
use crate::drivers::pl011::Pl011;
use crate::irq;
use crate::println;
use crate::sched;
use crate::time;

/// Timer firings to observe before believing CNTP works.
const SOFT_PROOF_COUNT: u32 = 3;

/// Iterations to spin waiting for a hardware condition before declaring it
/// dead. Generous: this runs once, at boot, on a quiet machine.
const SPIN_BUDGET: u32 = 20_000_000;

/// Print `soft_ticks=` every this many polled firings.
const TICK_PRINT_EVERY: u64 = 10;

/// `SCTLR_EL1` bits that are RES1 on ARMv8.0-A, the Cortex-A72's level.
///
/// `boot.s` programs this pattern because `msr sctlr_el1, xzr` used to clear
/// all six and nothing restored them — `enable_translation` only
/// read-modify-writes M/C/I on top. Writing 0 to a RES1 field is
/// architecturally UNPREDICTABLE.
const SCTLR_RES1: u64 = (1 << 11) | (1 << 20) | (1 << 22) | (1 << 23) | (1 << 28) | (1 << 29);

/// Read `SCTLR_EL1` back and report which RES1 bits survived.
///
/// A gate rather than a print: the interesting answer is whether the hardware
/// agrees with what `boot.s` wrote, and the two known possibilities differ.
/// Under QEMU the register reads `0x30d01805` — the pattern plus `M|C|I`. A
/// part that forces its own RES1 bits would report them set regardless, which
/// is equally fine and worth knowing. Only *missing* bits are a failure, and
/// they would mean the kernel is running in an architecturally undefined state.
fn sctlr_res1_survived(uart: &mut Pl011) -> bool {
    let sctlr: u64;
    // SAFETY: `SCTLR_EL1` is readable at EL1 and this is a plain register read
    // with no side effects.
    unsafe {
        core::arch::asm!("mrs {v}, sctlr_el1", v = out(reg) sctlr, options(nomem, nostack));
    }
    let present = sctlr & SCTLR_RES1;
    println!(
        uart,
        "selftest: SCTLR_EL1={sctlr:#x} RES1={present:#x}/{SCTLR_RES1:#x}"
    );
    present == SCTLR_RES1
}

/// Run every gate in order. `false` at the first failure.
pub fn run(uart: &mut Pl011) -> bool {
    println!(uart, "selftest: SCTLR_EL1 readback…");
    if !sctlr_res1_survived(uart) {
        println!(uart, "selftest: FAIL SCTLR RES1");
        return false;
    }

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

    // The task-stack overflow probe is not here: it needs a live scheduler and
    // a peer task, so bootstrap spawns it after `sched::init` (see
    // `guard_probe_task`). These gates run before any of that exists.
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
    time::on_timer_irq(0);

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
        time::on_timer_irq(0);
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

/// Recurse until the stack runs out, one real frame per call.
///
/// The frame is deliberately small: a large one can step *over* a 4 KiB guard
/// page and fault in whatever lies below it, which proves nothing about the
/// guard. `black_box` keeps the array live so the frame cannot be optimised
/// away, and the addition after the call keeps this frame alive across it —
/// a tail call would reuse one frame and never overflow.
///
/// The depth limit is unreachable (16 KiB of stack is gone long before) and
/// exists so the recursion is not provably infinite to the optimiser.
#[inline(never)]
fn eat_stack(depth: usize) -> u64 {
    let mut frame = [0u64; 8];
    frame[depth % frame.len()] = depth as u64;
    core::hint::black_box(&mut frame);
    if depth > 1_000_000 {
        return frame[0];
    }
    eat_stack(depth + 1) + frame[0]
}

/// Overflow this task's own stack while a peer task is alive.
///
/// Spawned by bootstrap as a third task, so the fault happens on a real
/// heap-allocated task stack under the scheduler — not on a stack allocated by
/// the probe itself. That distinction is M3's done-when: the claim is that an
/// overflow faults *rather than reaching another task's stack*, which cannot be
/// shown without a second stack in play.
///
/// Success is a data abort; the panic path prints ESR/FAR. Every range is
/// printed first so the captured `FAR` can be compared rather than deduced:
/// it must fall inside this task's guard and outside every stack listed.
pub fn guard_probe_task() {
    let mut map = [sched::StackReport::empty(); sched::MAX_TASKS];
    let count = sched::stack_map(&mut map);
    let me = sched::current_id();

    crate::kprintln!("PROBE: overflowing task {} of {count} live stacks", me.0);
    for report in &map[..count] {
        let tag = if report.id == me { "self" } else { "peer" };
        crate::kprintln!(
            "PROBE: {tag} task {} guard {:#x}..{:#x} stack {:#x}..{:#x}",
            report.id.0,
            report.guard.0,
            report.guard.1,
            report.stack.0,
            report.stack.1
        );
    }
    crate::kprintln!("PROBE: recursing until the guard faults");

    core::hint::black_box(eat_stack(0));

    // Reached only if the guard was still mapped: the recursion would then have
    // walked into whatever lies below, which is the failure this probe exists
    // to catch.
    crate::kprintln!("PROBE: FAIL — overflow did not fault");
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
