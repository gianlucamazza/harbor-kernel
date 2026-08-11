//! Deliberate fault, so the panic path has evidence it works (ADR-0093).
//!
//! Every other gate asserts that no boot printed `PANIC` — negative evidence,
//! and until this existed it was the only evidence `src/panic.rs` had
//! (excellence review F-24, deferred by ADR-0049). This image faults on
//! purpose and `scripts/boot/qemu-panic-boot-check.sh` asserts what comes out.
//!
//! ## Why a stack guard page
//!
//! It reaches the branch of `panic::report_faulting_address` with the most
//! policy in it — the one that calls a hole in the heap a *stack overflow*
//! rather than a wild pointer — and it is the first positive evidence that the
//! ADR-0005 guard faults at all. Every other gate only observes that nothing
//! has fallen into one.
//!
//! The stack is allocated by [`TaskStack::allocate`], the same function every
//! spawn uses, rather than by spawning a task and reading its geometry back:
//! the guard is the real mechanism either way, and this way the probe owes
//! nothing to the scheduler being up.
//!
//! ## Why it announces first
//!
//! Without the announce line, "the kernel did not panic" and "the probe was
//! never built in" produce the same log, and a gate that cannot tell those
//! apart passes on the day the probe silently stops running. The address it
//! prints is also what the gate compares against the `FAR` in the panic body —
//! that is how it knows the syndrome belongs to *this* fault.

use kernel_core::paging::PAGE_SIZE;

use crate::drivers::pl011::Pl011;
use crate::mm::TaskStack;
use crate::println;

/// Usable stack bytes to ask for. Three pages, the ordinary thin-stack size:
/// what matters is the guard page in front of it, not the size behind.
const USABLE_BYTES: usize = 3 * PAGE_SIZE as usize;

/// Try to fault by writing to a stack guard page.
///
/// In every healthy image this does not return: the store raises a data abort,
/// `exception_sync_el1` records the syndrome and panics, and the panic handler
/// halts the core. It is **not** declared `-> !` all the same, because the two
/// ways it can fail to fault — no stack, or a guard page that turns out to be
/// mapped — are exactly what a reader of this gate would want to see. Returning
/// lets the boot carry on and the gate report "announced, never panicked",
/// which is a better failure than a silent halt that looks like a hang.
pub fn fault_on_a_stack_guard(uart: &mut Pl011) {
    let stack = match TaskStack::allocate(USABLE_BYTES) {
        Ok(s) => s,
        Err(error) => {
            println!(uart, "panic-probe: could not allocate a stack: {error:?}");
            return;
        }
    };
    let Some((guard_low, _guard_high)) = stack.guard_range() else {
        println!(uart, "panic-probe: the stack has no guard page");
        return;
    };

    println!(uart, "panic-probe: stack guard at {guard_low:#x}, writing");

    // SAFETY: deliberately unsound — this is the point. The page was unmapped
    // by `TaskStack::allocate`, so the store must raise a data abort at EL1.
    unsafe { core::ptr::write_volatile(guard_low as *mut u64, 0xDEAD_BEEF) };

    // Reached only if the guard page was mapped after all, which would mean
    // ADR-0005's guard is not a guard. Say so; the gate fails on the missing
    // panic either way, and this line names the reason.
    println!(
        uart,
        "panic-probe: the write to {guard_low:#x} SUCCEEDED — the guard page is mapped"
    );
}
