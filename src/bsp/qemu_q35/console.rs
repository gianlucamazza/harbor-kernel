//! Bind COM1 for the lab console.

use crate::bsp::board::memmap::COM1_PORT;
use crate::drivers::uart16550::Uart16550;

/// # Safety
/// Call once on the primary lab path; COM1 is exclusive.
pub unsafe fn bind() -> Uart16550 {
    // SAFETY: QEMU ISA serial at COM1; port from board memmap.
    unsafe { Uart16550::new(COM1_PORT) }
}
