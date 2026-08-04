// Early AArch64 bootstrap for Raspberry Pi 4 Model B.
//
// Responsibilities (and nothing else):
//   1. Park secondary cores.
//   2. If entered at EL2 (typical with start4.elf), drop to EL1h.
//   3. Mask DAIF, clear BSS, install the stack, call kernel_main.
//   4. If kernel_main returns, park core 0.
//
// Symbols from the linker script: __bss_start, __bss_end, __stack_top.

.section .text.boot, "ax"
.global _start

_start:
    // Affinity level 0 only (core id within cluster).
    mrs     x0, mpidr_el1
    and     x0, x0, #0xFF
    cbz     x0, .L_primary

.L_park:
    wfe
    b       .L_park

.L_primary:
    // CurrentEL[3:2] — drop EL2 → EL1 when required.
    mrs     x0, CurrentEL
    lsr     x0, x0, #2
    cmp     x0, #2
    b.ne    .L_el1

    // EL1 is AArch64 (HCR_EL2.RW = 1).
    // IMO/FMO/AMO cleared so physical IRQ/FIQ/SError route to EL1, not EL2.
    msr     sctlr_el1, xzr
    mov     x0, #(1 << 31)
    msr     hcr_el2, x0

    // Allow EL1 to access the physical counter and timer (CNTHCTL_EL2).
    // EL1PCTEN (bit 0) + EL1PCEN (bit 1); clear virtual offset.
    mov     x0, #0x3
    msr     cnthctl_el2, x0
    msr     cntvoff_el2, xzr

    // Do not trap FP/SIMD to EL2 (CPTR_EL2.TFP = bit 10).
    mrs     x0, cptr_el2
    bic     x0, x0, #(1 << 10)
    msr     cptr_el2, x0

    // Ensure DAIF mask in SPSR before eret; drop to EL1h (SP_EL1).
    mov     x0, #0x3C5
    msr     spsr_el2, x0
    adr     x0, .L_el1
    msr     elr_el2, x0
    eret

.L_el1:
    msr     daifset, #0xF

    // Zero BSS: [__bss_start, __bss_end).
    ldr     x0, =__bss_start
    ldr     x1, =__bss_end
.L_bss:
    cmp     x0, x1
    b.hs    .L_stack
    str     xzr, [x0], #8
    b       .L_bss

.L_stack:
    ldr     x0, =__stack_top
    mov     sp, x0

    bl      kernel_main

    // kernel_main must not return; park if it does.
    b       .L_park
