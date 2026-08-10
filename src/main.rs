//! Kernel entry — assembly `_start` → `kernel_main` → product or lab path.
//!
//! Module layout follows the scale axes in
//! [`docs/design/project-topology.md`](../docs/design/project-topology.md):
//! ISA (`arch`), board (`bsp`), protocol (`drivers`), product policy, lab.

#![no_std]
#![no_main]

#[cfg(target_arch = "aarch64")]
extern crate alloc;

// --- Shared planes (every freestanding image) ---
mod arch;

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
mod bsp;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
mod drivers;

// --- Product path (AArch64 + board-rpi4) ---
#[cfg(target_arch = "aarch64")]
mod agent;
#[cfg(target_arch = "aarch64")]
mod bootstrap;
#[cfg(target_arch = "aarch64")]
mod console;
#[cfg(target_arch = "aarch64")]
mod durable;
#[cfg(target_arch = "aarch64")]
mod ipc;
#[cfg(target_arch = "aarch64")]
mod irq;
#[cfg(target_arch = "aarch64")]
mod mm;
#[cfg(target_arch = "aarch64")]
mod naming;
#[cfg(target_arch = "aarch64")]
mod panic;
#[cfg(target_arch = "aarch64")]
mod sched;
#[cfg(all(target_arch = "aarch64", feature = "debug-display"))]
mod status;
#[cfg(target_arch = "aarch64")]
mod storage;
#[cfg(target_arch = "aarch64")]
mod sync;
#[cfg(target_arch = "aarch64")]
mod taskcap;
#[cfg(target_arch = "aarch64")]
mod time;

// --- Lab path (thin bring-up; not product policy) ---
#[cfg(target_arch = "x86_64")]
mod lab;

/// Called from `_start` after early CPU setup.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_main() -> ! {
    #[cfg(target_arch = "aarch64")]
    {
        bootstrap::run()
    }
    #[cfg(target_arch = "x86_64")]
    {
        lab::run()
    }
}
