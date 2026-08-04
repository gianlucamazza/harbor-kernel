//! Panic handler.
//!
//! Masks IRQs, re-initialises the serial console, emits a diagnostic, then
//! parks the core. Unwinding is disabled (`panic = "abort"`).

use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::cpu;
use crate::console;

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
    let _ = writeln!(uart, "*** halt ***");

    cpu::halt()
}
