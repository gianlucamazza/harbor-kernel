//! Kernel entry — thin: assembly → bootstrap.

#![no_std]
#![no_main]

// The kernel heap backs `GlobalAlloc` (see `mm`), so `Box`, `Vec` and the rest
// of `alloc` are available to the rest of the kernel.
extern crate alloc;

mod arch;
mod bootstrap;
mod bsp;
mod console;
mod drivers;
mod irq;
mod mm;
mod panic;
mod sync;
mod time;

core::arch::global_asm!(include_str!("boot.s"));

/// Called from `_start` after EL1, BSS, stack.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_main() -> ! {
    bootstrap::run()
}
