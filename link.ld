/*
 * Linker script — Raspberry Pi 4 Model B, AArch64.
 *
 * The platform firmware (start4.elf) loads kernel8.img at physical 0x80000
 * and transfers control there. Layout:
 *
 *   0x80000  .text.boot  (_start)
 *            .text
 *            .rodata
 *            .data
 *            .bss
 *            [64 KiB stack, grows downward from __stack_top]
 */

ENTRY(_start)

SECTIONS
{
    . = 0x80000;

    .text : ALIGN(8) {
        KEEP(*(.text.boot))
        *(.text .text.*)
    }

    /* VBAR_EL1 requires 2 KiB alignment for the vector table. */
    .text.vectors : ALIGN(0x800) {
        KEEP(*(.text.vectors))
    }

    .rodata : ALIGN(8) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(8) {
        *(.data .data.*)
    }

    .bss (NOLOAD) : ALIGN(16) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(8);
        __bss_end = .;
    }

    /* Kernel stack: 64 KiB, full-descending, exclusive to core 0. */
    . = ALIGN(16);
    __stack_bottom = .;
    . = . + 0x10000;
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
