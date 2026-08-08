//! Kernel entry — thin: assembly → bootstrap.

#![no_std]
#![no_main]

// The kernel heap backs `GlobalAlloc` (see `mm`), so `Box`, `Vec` and the rest
// of `alloc` are available to the rest of the kernel.
extern crate alloc;

mod agent;
mod arch;
mod bootstrap;
mod bsp;
mod console;
mod drivers;
mod durable;
mod ipc;
mod irq;
mod mm;
mod naming;
mod panic;
mod sched;
#[cfg(feature = "debug-display")]
mod status;
mod storage;
mod sync;
mod time;

// Boot assembly is owned by the active ISA module (`arch`); see `arch/mod.rs`.

/// Called from `_start` after EL1, BSS, stack.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_main() -> ! {
    bootstrap::run()
}
