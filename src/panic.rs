//! Panic handler.
//!
//! Masks IRQs, re-initialises the serial console, emits a diagnostic, then
//! parks the core. Unwinding is disabled (`panic = "abort"`).

use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::cpu;
use crate::console;
use core::fmt::Write;
use core::panic::PanicInfo;

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

    #[cfg(feature = "debug-display")]
    {
        // Best-effort glass banner; UART already has the full diagnostic.
        let mut buf = [0u8; 60];
        let msg = info.message();
        let s = msg.as_str().unwrap_or("panic");
        let n = s.len().min(buf.len());
        buf[..n].copy_from_slice(s.as_bytes());
        let text = core::str::from_utf8(&buf[..n]).unwrap_or("panic");
        crate::status::show_panic(text);
    }

    cpu::halt()
}
