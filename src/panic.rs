//! Panic handler.
//!
//! Masks IRQs, re-initialises the serial console, emits a diagnostic, then
//! parks the core. Unwinding is disabled (`panic = "abort"`).

use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::cpu;
use crate::arch::exception;
use crate::console;
use crate::mm::layout::{self, AddressNote};
use core::fmt::Write;
use core::panic::PanicInfo;

/// Say *where* the faulting address was, when the panic came from a trap.
///
/// The handler already printed the syndrome in words, but `FAR` alone is a
/// number: on silicon there is no debugger to ask what lives there. The region
/// table knows, and it is policy's to read — so the trap publishes the
/// syndrome (`arch::exception::last_fault`) and this side names the address.
///
/// A guard page is called out by name rather than reported as unmapped: it is
/// unmapped *on purpose* (ADR-0005), so "stack overflow" and "wild pointer"
/// stop looking alike.
fn report_faulting_address(uart: &mut impl Write) {
    let Some((_esr, far, _seq)) = exception::last_fault() else {
        return;
    };
    match layout::describe_address(far) {
        AddressNote::In(name, perms) => {
            let _ = writeln!(
                uart,
                "fault: {far:#018x} in \"{name}\", mapped {}{}{}",
                if perms.write { "rw" } else { "ro" },
                if perms.execute { "x" } else { "" },
                if perms.user { " user" } else { "" }
            );
        }
        AddressNote::UnmappedInside(name) => {
            // The heap case is a task-stack guard: the page was unmapped after
            // the map was built, so the region table still calls it heap.
            let _ = writeln!(
                uart,
                "fault: {far:#018x} unmapped inside \"{name}\"{}",
                if name == "heap" {
                    " — task-stack guard page, i.e. stack overflow"
                } else {
                    ""
                }
            );
        }
        AddressNote::BootstrapGuard => {
            let _ = writeln!(
                uart,
                "fault: {far:#018x} is the bootstrap stack guard page — stack overflow"
            );
        }
        AddressNote::Unmapped => {
            let _ = writeln!(uart, "fault: {far:#018x} is in no mapped region");
        }
        AddressNote::MappedOutsideTable => {
            let _ = writeln!(
                uart,
                "fault: {far:#018x} translates but is in no kernel region"
            );
        }
        AddressNote::Unknown => {
            let _ = writeln!(uart, "fault: {far:#018x} (map not built yet)");
        }
    }
}

/// Set on entry so a panic raised *inside* the panic path does not recurse.
static PANICKING: AtomicBool = AtomicBool::new(false);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    cpu::irq_disable();

    // A second panic while reporting the first (console::acquire, formatting)
    // would otherwise loop here forever, spending the stack and printing
    // nothing useful. Park instead: the first message is already out.
    if PANICKING.swap(true, Ordering::Relaxed) {
        cpu::halt()
    }

    // SAFETY: the panicking context never resumes, so taking the console from
    // it is sound; re-init restores the UART from a cold programming state.
    let mut uart = unsafe { console::steal() };

    let _ = writeln!(uart, "\n*** KERNEL PANIC ***");
    let _ = writeln!(uart, "{info}");
    report_faulting_address(&mut uart);
    let _ = writeln!(uart, "*** halt ***");

    cpu::halt()
}
