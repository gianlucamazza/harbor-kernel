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
 *            64 KiB stack, grows down onto the guard     RW, no execute
 *            heap                                        RW, no execute
 */

ENTRY(_start)

/* One page of nothing between the stack and whatever precedes it: a stack
   overflow then faults instead of quietly eating the page tables. */
GUARD_PAGE_SIZE = 0x1000;

/* Root table + one L2 per mapped GiB + L3 tables for the sub-2 MiB detail.
   Four are needed today; sixteen leaves room to map more device windows
   without touching the linker script again. */
PAGE_TABLE_ARENA_SIZE = 16 * 0x1000;

STACK_SIZE = 0x10000;

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
    __guard_start = .;
    . = . + GUARD_PAGE_SIZE;
    __guard_end = .;

    /* Kernel stack: full-descending, exclusive to core 0. Overflowing it runs
       into the guard page above. */
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
