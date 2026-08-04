//! Kernel entry — thin: assembly → bootstrap.

#![no_std]
#![no_main]

mod arch;
mod bootstrap;
mod bsp;
mod console;
mod drivers;
mod irq;
mod mm;
mod panic;
mod time;

core::arch::global_asm!(include_str!("boot.s"));

/// Called from `_start` after EL1, BSS, stack.
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    bootstrap::run()
}
