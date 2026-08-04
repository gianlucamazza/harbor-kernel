//! Voluntary EL1 context switch (ADR-0006).
//!
//! Saves and restores the AAPCS64 callee-saved state plus `sp` and the
//! continuation in `x30`. Not a trap frame: IRQ entry still uses
//! [`super::exception::frame::TrapFrame`].

/// Callee-saved GPRs + link + stack pointer for a cooperative switch.
///
/// Layout is load-bearing for [`context_switch`].
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Context {
    /// `x19` … `x28`.
    pub x19_x28: [u64; 10],
    /// Frame pointer `x29`.
    pub x29: u64,
    /// Continuation / return address `x30`.
    pub x30: u64,
    /// Stack pointer.
    pub sp: u64,
}

impl Context {
    /// Zeroed context; `x30` and `sp` must be filled before first restore.
    pub const fn zeroed() -> Self {
        Self {
            x19_x28: [0; 10],
            x29: 0,
            x30: 0,
            sp: 0,
        }
    }
}

// The assembly below addresses these fields by hard-coded byte offset, so the
// struct and the offsets must be pinned together. Size alone is not enough:
// swapping `x29`/`x30`/`sp`, or splitting `x19_x28` into two arrays, keeps the
// size at 104 and silently switches to the wrong stack. `TrapFrame` carries the
// same four-assert pattern for the same reason, written after exactly that
// drift — see `exception/frame.rs`.
const _: () = assert!(core::mem::offset_of!(Context, x19_x28) == 0);
const _: () = assert!(core::mem::offset_of!(Context, x29) == 80);
const _: () = assert!(core::mem::offset_of!(Context, x30) == 88);
const _: () = assert!(core::mem::offset_of!(Context, sp) == 96);
const _: () = assert!(core::mem::size_of::<Context>() == 13 * 8);

core::arch::global_asm!(
    r#"
    .global context_switch
    // void context_switch(Context* prev, const Context* next)
    // x0 = prev, x1 = next
    .type context_switch, %function
    context_switch:
        // Save callee-saved of the outgoing task into *prev.
        stp x19, x20, [x0, #0]
        stp x21, x22, [x0, #16]
        stp x23, x24, [x0, #32]
        stp x25, x26, [x0, #48]
        stp x27, x28, [x0, #64]
        stp x29, x30, [x0, #80]
        mov x9, sp
        str x9, [x0, #96]

        // Restore callee-saved of the incoming task from *next.
        ldp x19, x20, [x1, #0]
        ldp x21, x22, [x1, #16]
        ldp x23, x24, [x1, #32]
        ldp x25, x26, [x1, #48]
        ldp x27, x28, [x1, #64]
        ldp x29, x30, [x1, #80]
        ldr x9, [x1, #96]
        mov sp, x9
        ret
    "#
);

unsafe extern "C" {
    /// Save into `prev`, restore from `next`, return into `next.x30`.
    ///
    /// # Safety
    /// Both pointers valid; IRQs masked; `next` describes a live stack.
    pub fn context_switch(prev: *mut Context, next: *const Context);
}
