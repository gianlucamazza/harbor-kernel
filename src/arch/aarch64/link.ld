/*
 * Linker script — Raspberry Pi 4 Model B, AArch64.
 *
 * The platform firmware (start4.elf) loads kernel8.img at physical 0x80000
 * and transfers control there.
 *
 * Every region boundary is page aligned so the MMU can give each one its own
 * permissions: code executable and read-only, everything else writable and
 * never executable. Without page-aligned boundaries a single 4 KiB page would
 * straddle .text and .data and the weaker permission would have to win.
 *
 *   0x80000  .text.boot (_start), .text, .text.vectors   RX
 *            .rodata                                     RO
 *            .data, .bss                                 RW, no execute
 *            translation table arena                     RW, no execute
 *            [guard page — deliberately never mapped]
 *            16 KiB exception stack (SP_EL1)             RW, no execute
 *            [guard page — deliberately never mapped]
 *            64 KiB kernel stack (SP_EL0), grows down    RW, no execute
 *            heap                                        RW, no execute
 */

ENTRY(_start)

/* One page of nothing between the stack and whatever precedes it: a stack
   overflow then faults instead of quietly eating the page tables. */
GUARD_PAGE_SIZE = 0x1000;

/* Root table + one L2 per mapped GiB + one L3 per 2 MiB window that is not a
   whole block. How many that is moves with `.text`, so it is not written down
   here — the boot line prints what is left, and `bootstrap` refuses to boot if
   the remainder falls under its reserve.

   Two things consume tables after that, both permanently: `mmu::map` for the
   DTB, and every `mmu::unmap` that has to split a 2 MiB block to punch out a
   task-stack guard. The arena is a bump allocator with no free path, so a
   split is never given back — spawning tasks into distinct blocks is what
   grows this number over a long uptime.

   The reserve is no longer a written-down number: `bootstrap` derives
   `MIN_SPARE_TABLES` from `sched::MAX_TASKS`, because the two had drifted to 4
   and 12 and the reserve was under-counting by 3×. At `MAX_TASKS = 12` that is
   14 tables, so the arena must hold what `activate` and the DTB consume (seven,
   as the boot line reports) plus those 14. Thirty-two leaves margin for the map
   growing with `.text`; `bootstrap::run` refuses to boot if it ever does not,
   and names this symbol when it does. */
PAGE_TABLE_ARENA_SIZE = 32 * 0x1000;

STACK_SIZE = 0x10000;

/* The exception stack only ever holds trap frames and the handlers above them,
   never the deep call chains the kernel stack sees. 16 KiB is roughly sixty
   nested frames of 272 bytes; nesting past that is a fault inside a fault,
   which this kernel does not try to survive. */
EXCEPTION_STACK_SIZE = 0x4000;

SECTIONS
{
    . = 0x80000;
    __image_start = .;

    .text : ALIGN(0x1000) {
        __text_start = .;
        KEEP(*(.text.boot))
        *(.text .text.*)

        /* VBAR_EL1 requires 2 KiB alignment. The section is named `.vectors`,
           not `.text.vectors`, precisely so the wildcard above cannot claim it
           first — which is what used to happen, leaving the alignment to the
           `.align 11` inside the assembly and nothing to enforce it here. */
        . = ALIGN(0x800);
        KEEP(*(.vectors))
    }
    . = ALIGN(0x1000);
    __text_end = .;

    .rodata : ALIGN(0x1000) {
        __rodata_start = .;
        *(.rodata .rodata.*)
        /* Agent composition store (ADR-0029). Inside the rodata VMA span so
           the fine RO map covers it. No Rust static: pure linker window so
           LTO cannot fold inject away. BYTE forces PROGBITS (not NOBITS). */
        . = ALIGN(0x1000);
        __agent_store_start = .;
        BYTE(0);
        . = __agent_store_start + 0x4000;
        __agent_store_end = .;
        . = ALIGN(0x1000);
        __rodata_end = .;
    }

    .data : ALIGN(0x1000) {
        __data_start = .;
        *(.data .data.*)
    }

    .bss (NOLOAD) : ALIGN(16) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(8);
        __bss_end = .;
    }
    . = ALIGN(0x1000);
    __data_end = .;

    /* Translation tables. Not in .bss: `boot.s` clears .bss before the MMU
       exists, and the paging code zeroes each table as it hands it out. */
    .pagetables (NOLOAD) : ALIGN(0x1000) {
        __pagetables_start = .;
        . = . + PAGE_TABLE_ARENA_SIZE;
        __pagetables_end = .;
    }

    /* Guard page: left out of every mapping on purpose. */
    . = ALIGN(0x1000);
    __exception_guard_start = .;
    . = . + GUARD_PAGE_SIZE;
    __exception_guard_end = .;

    /* Exception stack: SP_EL1, entered by hardware on every exception. Its own
       stack, and its own guard, because the overflow it has to survive is the
       kernel stack's: a handler that saved its trap frame onto the stack that
       just overflowed would fault again inside the fault. */
    __exception_stack_bottom = .;
    . = . + EXCEPTION_STACK_SIZE;
    __exception_stack_top = .;

    /* Guard page for the kernel stack. */
    . = ALIGN(0x1000);
    __guard_start = .;
    . = . + GUARD_PAGE_SIZE;
    __guard_end = .;

    /* Kernel stack: SP_EL0, full-descending, exclusive to core 0. Overflowing
       it runs into the guard page above. */
    __stack_bottom = .;
    . = . + STACK_SIZE;
    __stack_top = .;

    /* Heap starts on the next page after the stack (grows upward in RAM). */
    . = ALIGN(0x1000);
    __heap_start = .;

    /DISCARD/ : {
        *(.comment)
        *(.gnu*)
        *(.note*)
        *(.eh_frame*)
        *(.eh_frame_hdr*)
    }
}
