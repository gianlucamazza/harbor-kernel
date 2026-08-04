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
use kernel_core::paging::{self, MemKind, Perms};

use crate::arch::cpu;
use crate::arch::mmu;
use crate::console;
use crate::drivers::pl011::Pl011; // heap_check TX handle
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
            return;
        }
    }

    split_smoke(uart);
}

/// Unmap a page that is certainly inside a 2 MiB block, so the break-before-make
/// split runs on every boot.
///
/// Without this the split path is dead code in practice: `__heap_start` sits
/// below the first 2 MiB boundary, and `paging::chunks` degrades to 4 KiB pages
/// until it reaches one — so the heap head, where every task stack and the
/// smoke above land, is already page-granular. The guard mechanism therefore
/// works today by an accident of alignment, and the most delicate code in the
/// MMU would first execute in production, the day the heap fills past 2 MiB.
///
/// Reaching a block means allocating past the boundary rather than trusting one
/// to be nearby: the target page is computed from the allocation actually
/// returned, not assumed.
fn split_smoke(uart: &mut Pl011) {
    let block = paging::L2_BLOCK_SIZE as usize;
    // A block's worth plus a page guarantees the span contains a whole 2 MiB
    // boundary *and* a page above it, whatever the base alignment.
    let layout = match Layout::from_size_align(block + 0x1000, 0x1000) {
        Ok(layout) => layout,
        Err(_) => return,
    };

    // SAFETY: layout is non-zero and aligned; null means OOM.
    let ptr = unsafe { alloc(layout) };
    if ptr.is_null() {
        println!(uart, "split: SKIPPED (OOM)");
        return;
    }

    let base = ptr as u64;
    let target = base.next_multiple_of(paging::L2_BLOCK_SIZE);
    let before = mmu::splits();

    let unmapped = cpu::without_irqs(|| {
        // SAFETY: kernel map active; IRQs masked; page-aligned heap we own.
        unsafe { mmu::unmap(target, 0x1000) }
    });
    if let Err(error) = unmapped {
        println!(uart, "split: unmap FAILED {error:?}");
        // SAFETY: still fully mapped — unmap refused before changing anything.
        unsafe { dealloc(ptr, layout) };
        return;
    }

    let region = Region {
        base: target,
        len: 0x1000,
        kind: MemKind::NormalWb,
        perms: Perms::RW,
        name: "split smoke",
    };
    let remapped = cpu::without_irqs(|| {
        // SAFETY: IRQs masked; restoring the mapping we just removed.
        unsafe { mmu::map(&region) }
    });

    let splits = mmu::splits() - before;
    match remapped {
        Ok(()) => {
            // The page is live again: writing it proves the rebuilt table maps
            // the same physical memory, which a descriptor read alone cannot.
            // SAFETY: remapped RW immediately above.
            unsafe { core::ptr::write_volatile(target as *mut u8, 0xC3) };
            // SAFETY: fully mapped again; ownership returns to the allocator.
            unsafe { dealloc(ptr, layout) };
            println!(uart, "split: page at {target:#x} split {splits}, remapped");
        }
        Err(error) => {
            println!(uart, "split: remap FAILED {error:?}");
            // Intentionally leak: freeing an unmapped page would corrupt the heap.
        }
    }
}

/// Idle task body (ADR-0006): drain RX, report ticks, yield when others are
/// ready, otherwise `WFI`. TX goes through the shared handle from `install_tx`.
pub fn run() -> ! {
    let mut last_printed = 0u64;
    let mut last_counters = irq::Counters::default();
    let mut last_dropped = 0u32;
    let mut last_missed = 0u64;
    let mut last_abandoned = 0u32;

    loop {
        // 1. Echo all bytes the UART RX IRQ pushed into the ring.
        let _ = console::with_tx(|uart| {
            while let Some(byte) = console::pop_rx() {
                let sent = match byte {
                    b'\r' => uart.write_bytes(b"\r\n"),
                    byte => uart.write_byte(byte),
                };
                if !sent {
                    break;
                }
            }
        });

        // TFT status surface: rate-limited ticks/heap (never from IRQ).
        #[cfg(feature = "debug-display")]
        crate::status::on_idle();

        // 2. Periodic tick report, plus dispatch anomalies.
        let ticks = time::ticks();
        if ticks >= last_printed + TICK_PRINT_EVERY {
            let report = ticks - (ticks % TICK_PRINT_EVERY);
            if report > last_printed {
                let _ = console::with_tx(|uart| {
                    println!(uart, "ticks={report}");
                });
                last_printed = report;

                let counters = irq::counters();
                if counters != last_counters {
                    let _ = console::with_tx(|uart| {
                        println!(
                            uart,
                            "irq: unhandled={} out_of_range={} loop_exhausted={}",
                            counters.unhandled,
                            counters.out_of_range,
                            counters.loop_exhausted
                        );
                    });
                    last_counters = counters;
                }

                let dropped = console::rx_dropped();
                if dropped != last_dropped {
                    let _ = console::with_tx(|uart| {
                        println!(
                            uart,
                            "console: DROPPED {dropped} received bytes (ring full)"
                        );
                    });
                    last_dropped = dropped;
                }

                let missed = time::missed_ticks();
                if missed != last_missed {
                    let _ = console::with_tx(|uart| {
                        println!(uart, "timer: MISSED {missed} deadlines");
                    });
                    last_missed = missed;
                }

                // A task stack whose guard could not be remapped is leaked
                // rather than freed. Reported here because the alternative —
                // freeing it — corrupts the heap much later and elsewhere.
                let abandoned = crate::mm::task_stack::abandoned_stacks();
                if abandoned != last_abandoned {
                    let _ = console::with_tx(|uart| {
                        println!(uart, "sched: ABANDONED {abandoned} task stacks");
                    });
                    last_abandoned = abandoned;
                }
            }
        }

        // 3. Cooperative idle: run ready workers, else WFI without losing a wakeup.
        if crate::sched::has_ready() {
            crate::sched::yield_now();
            continue;
        }

        cpu::without_irqs(|| {
            if console::rx_is_empty() && time::ticks() < last_printed + TICK_PRINT_EVERY {
                cpu::wait_for_interrupt();
            }
        });
    }
}
