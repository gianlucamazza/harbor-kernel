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

.equ TRAP_FRAME_SIZE, 0x110

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

.section .text.vectors, "ax"
.align 11
.global exception_vectors
exception_vectors:
    // Current EL, SP_EL0 (EL1t) — unused (we run EL1h).
    ventry  exc_unexpected
    ventry  exc_unexpected
    ventry  exc_unexpected
    ventry  exc_unexpected

    // Current EL, SP_ELx (EL1h) — live kernel paths.
    ventry  exc_sync_el1h
    ventry  exc_irq_el1h
    ventry  exc_unexpected
    ventry  exc_unexpected

    // Lower EL, AArch64.
    ventry  exc_unexpected
    ventry  exc_unexpected
    ventry  exc_unexpected
    ventry  exc_unexpected

    // Lower EL, AArch32.
    ventry  exc_unexpected
    ventry  exc_unexpected
    ventry  exc_unexpected
    ventry  exc_unexpected

exc_sync_el1h:
    kernel_entry
    mov     x0, sp
    bl      exception_sync_el1
    kernel_exit

exc_irq_el1h:
    kernel_entry
    bl      exception_irq_el1
    kernel_exit

exc_unexpected:
    kernel_entry
    mov     x0, sp
    bl      exception_unexpected
    // exception_unexpected does not return; park if it ever does.
1:
    wfe
    b       1b
