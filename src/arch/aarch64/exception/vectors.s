// AArch64 exception vector table for EL1.
//
// VBAR_EL1 requires 2 KiB alignment. Each entry is 0x80 bytes (128).
// Groups: EL1t, EL1h, EL0_64, EL0_32 × {Sync, IRQ, FIQ, SError}.
//
// Trap frame (matches TrapFrame in frame.rs), 272 bytes on stack (16-aligned):
//   [sp+0x00] x0..x29   (15 × stp)
//   [sp+0xF0] x30
//   [sp+0xF8] elr
//   [sp+0x100] spsr
//   [sp+0x108] pad
// Total 0x110 = 272.

// TRAP_FRAME_SIZE is substituted by `global_asm!` from `frame.rs`, so the
// struct and the stack reservation cannot disagree.

.macro kernel_entry
    sub     sp, sp, #TRAP_FRAME_SIZE
    stp     x0,  x1,  [sp, #0x00]
    stp     x2,  x3,  [sp, #0x10]
    stp     x4,  x5,  [sp, #0x20]
    stp     x6,  x7,  [sp, #0x30]
    stp     x8,  x9,  [sp, #0x40]
    stp     x10, x11, [sp, #0x50]
    stp     x12, x13, [sp, #0x60]
    stp     x14, x15, [sp, #0x70]
    stp     x16, x17, [sp, #0x80]
    stp     x18, x19, [sp, #0x90]
    stp     x20, x21, [sp, #0xA0]
    stp     x22, x23, [sp, #0xB0]
    stp     x24, x25, [sp, #0xC0]
    stp     x26, x27, [sp, #0xD0]
    stp     x28, x29, [sp, #0xE0]
    mrs     x0,  elr_el1
    mrs     x1,  spsr_el1
    stp     x30, x0,  [sp, #0xF0]
    str     x1,       [sp, #0x100]
.endm

.macro kernel_exit
    ldr     x1,       [sp, #0x100]
    ldp     x30, x0,  [sp, #0xF0]
    msr     elr_el1,  x0
    msr     spsr_el1, x1
    ldp     x0,  x1,  [sp, #0x00]
    ldp     x2,  x3,  [sp, #0x10]
    ldp     x4,  x5,  [sp, #0x20]
    ldp     x6,  x7,  [sp, #0x30]
    ldp     x8,  x9,  [sp, #0x40]
    ldp     x10, x11, [sp, #0x50]
    ldp     x12, x13, [sp, #0x60]
    ldp     x14, x15, [sp, #0x70]
    ldp     x16, x17, [sp, #0x80]
    ldp     x18, x19, [sp, #0x90]
    ldp     x20, x21, [sp, #0xA0]
    ldp     x22, x23, [sp, #0xB0]
    ldp     x24, x25, [sp, #0xC0]
    ldp     x26, x27, [sp, #0xD0]
    ldp     x28, x29, [sp, #0xE0]
    add     sp, sp, #TRAP_FRAME_SIZE
    eret
.endm

// Branch to label; pad entry to 0x80 bytes.
.macro ventry label
    .align 7
    b       \label
.endm

.section .vectors, "ax"
.align 11
.global exception_vectors
exception_vectors:
    // Current EL, SP_EL0 (EL1t) — the live kernel paths. The kernel runs on
    // SP_EL0 (boot.s sets SPSel=0) precisely so that taking an exception
    // switches to SP_EL1, a stack of its own: a kernel stack overflow can then
    // be reported instead of faulting again inside the handler.
    ventry  exc_sync_el1t
    ventry  exc_irq_el1t
    ventry  exc_unexpected
    ventry  exc_unexpected

    // Current EL, SP_ELx (EL1h) — reached only from inside a handler, which is
    // already running on SP_EL1. A fault there is a fault inside a fault; this
    // kernel reports it and stops rather than pretending to recover.
    ventry  exc_unexpected
    ventry  exc_unexpected
    ventry  exc_unexpected
    ventry  exc_unexpected

    // Lower EL, AArch64 (EL0 → EL1).
    ventry  exc_sync_el0
    ventry  exc_irq_el0
    ventry  exc_lower_unexpected
    ventry  exc_lower_unexpected

    // Lower EL, AArch32 (unsupported — same restore-then-panic path).
    ventry  exc_lower_unexpected
    ventry  exc_lower_unexpected
    ventry  exc_lower_unexpected
    ventry  exc_lower_unexpected

exc_sync_el1t:
    kernel_entry
    mov     x0, sp
    bl      exception_sync_el1
    kernel_exit

exc_irq_el1t:
    kernel_entry
    bl      exception_irq_el1
    kernel_exit

// ADR-0014: never call switch_ttbr0 with a null root.
// Live el0 session → the published El0Session (ADR-0017 §1) holds the kernel
// root to reinstall. Two ways to have none: no session published for the
// running task, or one published with the root cleared. Both mean the same
// thing here — nobody owns this lower-EL event — and both take the same exit.
.macro restore_kernel_ttbr0_require_session
    adrp    x16, CURRENT_EL0
    add     x16, x16, :lo12:CURRENT_EL0
    ldr     x16, [x16]
    cbz     x16, el0_missing_kernel_ttbr
    ldr     x0, [x16, #EL0S_SESSION_KERNEL_TTBR]
    cbz     x0, el0_missing_kernel_ttbr
    bl      switch_ttbr0
.endm

// Optional restore: only if a session published a root (never switch to 0).
.macro restore_kernel_ttbr0_if_session
    adrp    x16, CURRENT_EL0
    add     x16, x16, :lo12:CURRENT_EL0
    ldr     x16, [x16]
    cbz     x16, 1f
    ldr     x0, [x16, #EL0S_SESSION_KERNEL_TTBR]
    cbz     x0, 1f
    bl      switch_ttbr0
1:
.endm

// Lower-EL sync: save frame under user TTBR (exception stack is cloned), then
// require session root and switch before handler C.
exc_sync_el0:
    kernel_entry
    restore_kernel_ttbr0_require_session
    mov     x0, sp
    bl      exception_sync_el0
    // Drop the trap frame so SP_EL1 does not walk down the exception stack
    // across one-shot sessions (finish switches to the el0_run frame).
    add     sp, sp, #TRAP_FRAME_SIZE
    b       el0_run_finish

exc_irq_el0:
    kernel_entry
    restore_kernel_ttbr0_require_session
    mov     x0, sp
    bl      exception_irq_el0
    // Same epilogue as sync: drop trap frame, return into el0_run / el0_resume.
    add     sp, sp, #TRAP_FRAME_SIZE
    b       el0_run_finish

// FIQ / SError (and AArch32) from lower EL: restore if a session is live so
// the panic path runs under the kernel root, then same unexpected handler.
exc_lower_unexpected:
    kernel_entry
    restore_kernel_ttbr0_if_session
    mov     x0, sp
    bl      exception_unexpected
1:
    wfe
    b       1b

// Same-EL unexpected (EL1t FIQ/SError, EL1h): already on kernel TTBR0.
exc_unexpected:
    kernel_entry
    mov     x0, sp
    bl      exception_unexpected
    // exception_unexpected does not return; park if it ever does.
1:
    wfe
    b       1b
