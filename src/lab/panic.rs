//! Panic handler for lab images (COM1 bind).
//!
//! Same **role** as product `crate::panic` (progressive-isa P.10); different
//! console bind. Lives under `lab/` so product panic stays board-PL011-centric.

use core::fmt::Write;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::cpu;
use crate::bsp::board;

static PANICKING: AtomicBool = AtomicBool::new(false);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    cpu::irq_disable();
    if PANICKING.swap(true, Ordering::Relaxed) {
        cpu::halt()
    }
    // SAFETY: panicking context; re-init COM1.
    let mut uart = unsafe { board::console::bind() };
    let _ = writeln!(uart, "\n*** KERNEL PANIC ***");
    let _ = writeln!(uart, "{info}");
    let _ = writeln!(uart, "*** halt ***");
    cpu::halt()
}
