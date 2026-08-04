//! The interactive console loop — the closest thing this kernel has to an
//! application, kept out of the bring-up sequence that starts it.
//!
//! Until agents exist, this is where "what the machine does" lives: echo what
//! arrives on the serial line, report the tick counter, and sleep otherwise.

use alloc::alloc::{alloc, dealloc};
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::alloc::Layout;

use kernel_core::layout::Region;
use kernel_core::paging::{MemKind, Perms};

use crate::arch::cpu;
use crate::arch::mmu;
use crate::console;
use crate::drivers::pl011::Pl011;
use crate::irq;
use crate::mm;
use crate::println;
use crate::time;

/// Print `ticks=` every this many IRQ ticks.
const TICK_PRINT_EVERY: u64 = 10;

/// Exercise the global allocator and report whether memory comes back.
///
/// The interesting number is the one after the drop: a bump allocator prints a
/// smaller figure there, a real one prints the figure it started with.
pub fn heap_check(uart: &mut Pl011) {
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

    // A refusal here means something freed a pointer it did not own. The heap
    // survived it — that is what refusing buys — but the caller is wrong, and
    // silence would let it stay wrong. The boot check fails on this line.
    let refused = mm::refused_frees();
    if refused != 0 {
        println!(uart, "heap: REFUSED {refused} invalid frees");
    }

    unmap_smoke(uart);
}

/// Exercise [`mmu::unmap`] on a heap page (and the block-split path when the
/// heap sits under a 2 MiB leaf). Remaps before free so the free-list never
/// sees a virtual hole. A fault here would hang the boot — that is the point.
fn unmap_smoke(uart: &mut Pl011) {
    // Two pages: first becomes a temporary guard, second stays mapped so we can
    // still prove neighbouring memory survived the split.
    let layout = match Layout::from_size_align(0x2000, 0x1000) {
        Ok(layout) => layout,
        Err(_) => return,
    };

    // SAFETY: layout is non-zero and aligned; null means OOM.
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        println!(uart, "unmap: SKIPPED (OOM)");
        return;
    }
    let base = ptr as u64;

    // SAFETY: freshly allocated, mapped writable by the heap region.
    unsafe {
        core::ptr::write_bytes(ptr, 0x5A, 0x2000);
    }

    let unmapped = cpu::without_irqs(|| {
        // SAFETY: kernel map active; IRQs masked; range is page-aligned heap.
        unsafe { mmu::unmap(base, 0x1000) }
    });

    match unmapped {
        Ok(()) => {
            // Neighbour page must still be live after a possible L2→L3 split.
            // SAFETY: second page was not unmapped.
            unsafe {
                core::ptr::write_volatile(ptr.add(0x1000), 0xA5);
            }
            println!(uart, "unmap: page at {base:#x} fault-ready");
        }
        Err(error) => {
            println!(uart, "unmap: FAILED {error:?}");
            // SAFETY: allocation still fully mapped on the failure path.
            unsafe { dealloc(ptr, layout) };
            return;
        }
    }

    let region = Region {
        base,
        len: 0x1000,
        kind: MemKind::NormalWb,
        perms: Perms::RW,
        name: "unmap smoke",
    };
    let remapped = cpu::without_irqs(|| {
        // SAFETY: IRQs masked; restoring the mapping we just removed.
        unsafe { mmu::map(&region) }
    });

    match remapped {
        Ok(()) => {
            // SAFETY: both pages mapped again; ownership returns to the allocator.
            unsafe { dealloc(ptr, layout) };
            println!(uart, "unmap: remapped and freed");
        }
        Err(error) => {
            println!(uart, "unmap: remap FAILED {error:?}");
            // Intentionally leak: freeing an unmapped page would corrupt the heap.
        }
    }
}

/// Event-driven console: drain the RX ring, report ticks, idle with `WFI`.
pub fn run(uart: &mut Pl011) -> ! {
    let mut last_printed = 0u64;
    let mut last_counters = irq::Counters::default();
    let mut last_dropped = 0u32;
    let mut last_missed = 0u64;

    loop {
        // 1. Echo all bytes the UART RX IRQ pushed into the ring.
        while let Some(byte) = console::pop_rx() {
            let sent = match byte {
                b'\r' => uart.write_bytes(b"\r\n"),
                byte => uart.write_byte(byte),
            };
            if !sent {
                // The transmitter stopped draining. Keep draining the ring so
                // a level-triggered RX line does not re-fire forever, but stop
                // echoing: there is nowhere for the bytes to go.
                break;
            }
        }

        // 2. Periodic tick report, plus dispatch anomalies. Reporting the
        //    counters only when they move keeps a quiet system quiet while
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

                // The RX handler cannot report its own losses: it is forbidden
                // from transmitting. It counts, and this is where the count is
                // told. Reported only when it moves, like the counters above.
                let dropped = console::rx_dropped();
                if dropped != last_dropped {
                    println!(
                        uart,
                        "console: DROPPED {dropped} received bytes (ring full)"
                    );
                    last_dropped = dropped;
                }

                // Deadlines nobody was there to serve. Zero on a healthy
                // system; anything else means the handler ran late, which is
                // what a scheduler on this clock would trip over first.
                let missed = time::missed_ticks();
                if missed != last_missed {
                    println!(uart, "timer: MISSED {missed} deadlines");
                    last_missed = missed;
                }
            }
        }

        // 3. Idle when nothing is pending. Timer and UART RX both wake us.
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
