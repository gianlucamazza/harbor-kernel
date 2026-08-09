// Early AArch64 bootstrap for Raspberry Pi 4 Model B.
//
// Responsibilities (and nothing else):
//   1. Park secondary cores until `secondary_entry[aff]` is set (ADR-0070).
//   2. If entered at EL2 (typical with start4.elf), drop to EL1h.
//   3. Mask DAIF, clear BSS, install the stack, call kernel_main.
//   4. If kernel_main returns, park core 0.
//
// Symbols from the linker script: __bss_start, __bss_end, __stack_top.
// Core1 stacks: __core1_exception_stack_top / __core1_stack_top (from smp.rs).

// Storage for the firmware's x0. Deliberately in .data, not .bss: the BSS
// clear below runs *after* this is written and would zero it.
.section .data
.global __dtb_ptr
.align 3
__dtb_ptr:
    .quad 0

// `secondary_entry` lives in Rust (`arch::smp`) as `[AtomicU64; 4]`.

.section .text.boot, "ax"
.global _start
// Give the symbol a type and a size: the linker merges `.text.boot` into
// `.text`, so `scripts/check-pre-mmu-path.sh` can only find the entry code by
// symbol, and a sizeless NOTYPE symbol leaves it guessing where it ends.
.type _start, %function

_start:
    // The firmware passes the device-tree blob address in x0. `bootinfo`
    // validates it and `bootstrap` maps it read-only after the kernel map is
    // active; nothing *parses* it yet, but it is the only route to discovering
    // RAM size, the UART clock and the peripheral base instead of hard-coding
    // them — and it is unrecoverable once this register is reused.
    adrp    x19, __dtb_ptr
    add     x19, x19, :lo12:__dtb_ptr
    str     x0, [x19]

    // Affinity level 0 only (core id within cluster).
    mrs     x0, mpidr_el1
    and     x0, x0, #0xFF
    // Direct branch only: secondary wait has an indirect `br` and must not
    // sit inside `_start`'s pre-MMU audit window (scripts/check/pre-mmu-path.sh).
    cbnz    x0, secondary_wait

.L_primary:
    // CurrentEL[3:2]. Only EL2 and EL1 are handled: platform firmware
    // (start4.elf) enters at EL2, and a firmware that has already dropped to
    // EL1 is equally fine. EL3 is neither — the code below would program
    // EL1/EL2 state that has no effect from there and then `eret` into a
    // configuration nobody set up. This used to fall through to `.L_el1` and
    // continue as though it were already at EL1, which is a wrong answer rather
    // than a missing one, so it parks instead. There is no console yet to say
    // why; `docs/boot-chain.md` carries the note.
    mrs     x0, CurrentEL
    lsr     x0, x0, #2
    cmp     x0, #2
    b.gt    .L_park
    b.lt    .L_el1

    // SCTLR_EL1 starts from a known state — but not from zero. Bits 11, 20, 22,
    // 23, 28 and 29 are RES1 on ARMv8.0-A (the Cortex-A72's architecture), and
    // writing 0 to a RES1 field is UNPREDICTABLE. `msr sctlr_el1, xzr` cleared
    // all six, and nothing put them back: `enable_translation` only
    // read-modify-writes M/C/I on top, so the kernel ran with SCTLR_EL1 =
    // 0x1005, measured under QEMU. The reset value would have had them set;
    // this write is what took them away, so this write is what restores them.
    ldr     x0, =0x30d00800
    msr     sctlr_el1, x0

    // EL1 is AArch64 (HCR_EL2.RW = 1).
    // IMO/FMO/AMO cleared so physical IRQ/FIQ/SError route to EL1, not EL2.
    mov     x0, #(1 << 31)
    msr     hcr_el2, x0

    // Allow EL1 to access the physical counter and timer (CNTHCTL_EL2).
    // EL1PCTEN (bit 0) + EL1PCEN (bit 1); clear virtual offset.
    mov     x0, #0x3
    msr     cnthctl_el2, x0
    msr     cntvoff_el2, xzr

    // Do not trap FP/SIMD to EL2 (CPTR_EL2.TFP = bit 10). Not a step towards
    // using FP — the kernel is softfloat and `CPACR_EL1.FPEN` is left trapping
    // on purpose. It is about *where* a stray FP instruction lands: with TFP
    // set the trap is taken to EL2, which after the `eret` below has no vector
    // table installed and no code to run. Clearing it means such a fault
    // reaches the EL1 handler, which can name it, rather than a level this
    // kernel has abandoned.
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
    // Two stacks. Exceptions are taken with PSTATE.SP forced to 1, so SP_EL1 is
    // the stack the hardware switches to on entry — give it its own, and run
    // the kernel on SP_EL0 instead. That is what makes a kernel stack overflow
    // reportable: the handler saves its trap frame somewhere else, instead of
    // below the overflow, where it would fault again and hang.
    //
    // Still EL1h here, so `sp` is SP_EL1.
    ldr     x0, =__exception_stack_top
    mov     sp, x0

    // Kernel stack on SP_EL0, then switch to it. From here on `sp` is SP_EL0
    // and exceptions vector through the EL1t entries.
    ldr     x0, =__stack_top
    msr     sp_el0, x0
    msr     spsel, #0

    // Enable translation with the compile-time identity map *before* any other
    // Rust runs. Until this point memory has no attributes, and an atomic
    // read-modify-write there spins forever on Cortex-A72 — a whole class of
    // silent hangs that no emulator reproduces. See arch::aarch64::mmu.
    bl      early_mmu_enable

    bl      kernel_main

    // kernel_main must not return; park if it does.
    b       .L_park

.L_park:
    wfe
    b       .L_park

.size _start, . - _start

// --- Secondaries (outside `_start` size: may use indirect branches) ---

// Wait for a non-zero entry in `secondary_entry[aff0]` (path A, real Pi).
.global secondary_wait
.type secondary_wait, %function
secondary_wait:
    adrp    x1, secondary_seen
    add     x1, x1, :lo12:secondary_seen
    ldr     x2, [x1]
    add     x2, x2, #1
    str     x2, [x1]
    mrs     x0, mpidr_el1
    and     x0, x0, #0xFF
    cmp     x0, #3
    b.hi    park_forever
    adrp    x1, secondary_entry
    add     x1, x1, :lo12:secondary_entry
    ldr     x2, [x1, x0, lsl #3]
    cbz     x2, 1f
    br      x2
1:
    wfe
    b       secondary_wait
.size secondary_wait, . - secondary_wait

.global park_forever
.type park_forever, %function
park_forever:
    wfe
    b       park_forever
.size park_forever, . - park_forever

// Core 1 entry: path A (table) or path B (QEMU spin-table at 0xe0).
.global secondary_el2_entry
.type secondary_el2_entry, %function
secondary_el2_entry:
    mrs     x0, CurrentEL
    lsr     x0, x0, #2
    cmp     x0, #2
    b.gt    park_forever
    b.lt    .L_secondary_el1

    ldr     x0, =0x30d00800
    msr     sctlr_el1, x0
    mov     x0, #(1 << 31)
    msr     hcr_el2, x0
    mov     x0, #0x3
    msr     cnthctl_el2, x0
    msr     cntvoff_el2, xzr
    mrs     x0, cptr_el2
    bic     x0, x0, #(1 << 10)
    msr     cptr_el2, x0
    mov     x0, #0x3C5
    msr     spsr_el2, x0
    adr     x0, .L_secondary_el1
    msr     elr_el2, x0
    eret

.L_secondary_el1:
    msr     daifset, #0xF
    adrp    x0, CORE1_EXC_STACK
    add     x0, x0, :lo12:CORE1_EXC_STACK
    add     x0, x0, #(16 * 1024)
    mov     sp, x0
    adrp    x0, CORE1_KER_STACK
    add     x0, x0, :lo12:CORE1_KER_STACK
    add     x0, x0, #(16 * 1024)
    msr     sp_el0, x0
    msr     spsel, #0
    bl      secondary_main
    b       park_forever
.size secondary_el2_entry, . - secondary_el2_entry
