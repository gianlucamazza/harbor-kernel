//! Kernel console policy.
//!
//! Until a resident console agent exists, callers hold an explicit [`Pl011`]
//! obtained via [`acquire`]. Formatting helpers take that handle as the first
//! argument so ownership stays visible and testable.

use crate::bsp::board;
use crate::drivers::pl011::Pl011;

/// Bring up the board serial console and return exclusive ownership of it.
///
/// Each call re-programs pinmux and the UART from a known reset state. That
/// makes the operation valid both on the cold boot path and in the panic
/// handler (where any previous handle may already be invalid).
///
/// # Safety
///
/// The caller must guarantee exclusive ownership of the console hardware
/// (GPIO pinmux for the UART pins and the PL011 MMIO block) for the entire
/// lifetime of the returned handle. On Milestone 0 this is satisfied by:
/// single active core, interrupts masked, no other subsystem touching UART.
pub unsafe fn acquire() -> Pl011 {
    board::console::init()
}

/// Write formatted output to a caller-owned console.
#[macro_export]
macro_rules! print {
    ($uart:expr, $($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = core::write!($uart, $($arg)*);
    }};
}

/// Write a line of formatted output to a caller-owned console.
#[macro_export]
macro_rules! println {
    ($uart:expr) => {{
        $crate::print!($uart, "\n");
    }};
    ($uart:expr, $($arg:tt)*) => {{
        $crate::print!($uart, "{}\n", format_args!($($arg)*));
    }};
}
