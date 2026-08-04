//! Panic handler.
//!
//! Masks IRQs, re-initialises the serial console, emits a diagnostic, then
//! parks the core. Unwinding is disabled (`panic = "abort"`).

use core::fmt::Write;
use core::panic::PanicInfo;

use crate::arch::cpu;
use crate::console;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    cpu::irq_disable();

    // SAFETY: single execution context after panic on core 0; re-init restores
    // the UART from a cold programming state.
    let mut uart = unsafe { console::acquire() };

    let _ = writeln!(uart, "\n*** KERNEL PANIC ***");
    let _ = writeln!(uart, "{info}");
    let _ = writeln!(uart, "*** halt ***");

    cpu::halt()
}
