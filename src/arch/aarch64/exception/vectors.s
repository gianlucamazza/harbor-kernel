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
    // ADR-0068: after claim → dispatch → EOI, ask sched whether the
    // current task's quantum expired. Pure predicate — the switch itself
    // never runs on the exception stack.
    bl      el1_preempt_pending
    cbz     w0, 1f
    // `b`, not `bl`: the pivot ends in its own eret and never returns here.
    b       el1_preempt_pivot
1:
    kernel_exit

// ADR-0068: rotate the interrupted EL1 task from the IRQ-return epilogue.
//
// Entry: SPSel=1, sp = the trap frame kernel_entry pushed on the exception
// stack, DAIF fully masked (exception entry), every GPR dead (saved in the
// frame — this code may clobber anything). The interrupted task's stack
// pointer is intact in SP_EL0, which EL1 can read and write while running
// on SP_EL1 — so the frame is copied onto the task's own stack *before*
// the exception stack is unwound, and no live data ever sits above
// SP_EL1's top.
//
// After the pivot, `el1_preempt_from_irq` runs an ordinary
// switch_with(Preempt) on the task's own stack; its saved continuation is
// simply the tail of this routine, so resume needs no new scheduler mode:
// restore the frame from our own sp and eret. DAIF stays masked from here
// to the eret; the SPSR restore reopens the interrupted task's I bit.
el1_preempt_pivot:
    mrs     x1, sp_el0
    sub     x1, x1, #TRAP_FRAME_SIZE
    ldp     x2, x3, [sp, #0x00]
    stp     x2, x3, [x1, #0x00]
    ldp     x2, x3, [sp, #0x10]
    stp     x2, x3, [x1, #0x10]
    ldp     x2, x3, [sp, #0x20]
    stp     x2, x3, [x1, #0x20]
    ldp     x2, x3, [sp, #0x30]
    stp     x2, x3, [x1, #0x30]
    ldp     x2, x3, [sp, #0x40]
    stp     x2, x3, [x1, #0x40]
    ldp     x2, x3, [sp, #0x50]
    stp     x2, x3, [x1, #0x50]
    ldp     x2, x3, [sp, #0x60]
    stp     x2, x3, [x1, #0x60]
    ldp     x2, x3, [sp, #0x70]
    stp     x2, x3, [x1, #0x70]
    ldp     x2, x3, [sp, #0x80]
    stp     x2, x3, [x1, #0x80]
    ldp     x2, x3, [sp, #0x90]
    stp     x2, x3, [x1, #0x90]
    ldp     x2, x3, [sp, #0xA0]
    stp     x2, x3, [x1, #0xA0]
    ldp     x2, x3, [sp, #0xB0]
    stp     x2, x3, [x1, #0xB0]
    ldp     x2, x3, [sp, #0xC0]
    stp     x2, x3, [x1, #0xC0]
    ldp     x2, x3, [sp, #0xD0]
    stp     x2, x3, [x1, #0xD0]
    ldp     x2, x3, [sp, #0xE0]
    stp     x2, x3, [x1, #0xE0]
    ldp     x2, x3, [sp, #0xF0]
    stp     x2, x3, [x1, #0xF0]
    ldp     x2, x3, [sp, #0x100]
    stp     x2, x3, [x1, #0x100]
    msr     sp_el0, x1
    add     sp, sp, #TRAP_FRAME_SIZE
    msr     spsel, #0
    bl      el1_preempt_from_irq
    kernel_exit

// ADR-0014: never call switch_ttbr0 with a null root.
// Live el0 session → this core's published El0Session (ADR-0017 §1 / ADR-0081)
// holds the kernel root to reinstall. Index CURRENT_EL0[MPIDR.Aff0].
// Two ways to have none: no session published for the running task on this
// core, or one published with the root cleared. Both mean the same thing
// here — nobody owns this lower-EL event — and both take the same exit.
// Clobbers x16, x17.
.macro restore_kernel_ttbr0_require_session
    mrs     x17, mpidr_el1
    and     x17, x17, #0xff
    cmp     x17, #2
    b.lo    2f
    mov     x17, xzr
2:
    adrp    x16, CURRENT_EL0
    add     x16, x16, :lo12:CURRENT_EL0
    ldr     x16, [x16, x17, lsl #3]
    cbz     x16, el0_no_live_session
    ldr     x0, [x16, #EL0S_SESSION_KERNEL_TTBR]
    cbz     x0, el0_no_live_session
    bl      switch_ttbr0
.endm

// Optional restore: only if this core's session published a root (never 0).
// Clobbers x16, x17.
.macro restore_kernel_ttbr0_if_session
    mrs     x17, mpidr_el1
    and     x17, x17, #0xff
    cmp     x17, #2
    b.lo    2f
    mov     x17, xzr
2:
    adrp    x16, CURRENT_EL0
    add     x16, x16, :lo12:CURRENT_EL0
    ldr     x16, [x16, x17, lsl #3]
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
