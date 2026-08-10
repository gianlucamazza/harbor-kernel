/* Harbor x86_64 lab image (ADR-0071). Load at 1 MiB; QEMU -kernel + PVH note.
 *
 * small code model + static reloc; image base matches VMA (progressive-isa B.4–B.5).
 */

ENTRY(_start)

KERNEL_VMA = 0x00100000;

PHDRS
{
    text PT_LOAD FLAGS(5);   /* R E */
    data PT_LOAD FLAGS(6);   /* RW */
    note PT_NOTE FLAGS(4);   /* R */
}

SECTIONS
{
    . = KERNEL_VMA;

    .boot : ALIGN(4K) {
        *(.text.boot)
    } :text

    .text : ALIGN(4K) {
        *(.text .text.*)
    } :text

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    } :text

    /* PVH note — required for QEMU ELF64 -kernel. */
    .note.Xen : ALIGN(4) {
        KEEP(*(.note.Xen))
    } :note :text

    .data : ALIGN(4K) {
        *(.data .data.*)
    } :data

    /* Boot page tables (zeroed and filled in boot.s; not part of .bss). */
    .boot.pt (NOLOAD) : ALIGN(4K) {
        *(.boot.pt)
    } :data

    .bss (NOLOAD) : ALIGN(16) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    } :data

    /* Primary kernel stack (grows down); in RW PT_LOAD so MemSize is honest. */
    .stack (NOLOAD) : ALIGN(4K) {
        . += 0x10000;
        __stack_top = .;
    } :data

    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment)
    }
}
