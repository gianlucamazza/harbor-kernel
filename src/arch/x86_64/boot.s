/* x86_64 freestanding entry (ADR-0071). AT syntax.
 *
 * Loader: QEMU `-kernel` ELF64 via PVH note XEN_ELFNOTE_PHYS32_ENTRY (type 18).
 * That is the protocol QEMU implements for 64-bit freestanding images — not a
 * Multiboot fallback and not a Linux EFI stub (docs/design/progressive-isa-practices.md).
 *
 * Construction:
 *  - Boot page tables live in `.boot.pt` (NOLOAD), zeroed then filled here.
 *  - `.bss` is zeroed while still identity-physical, before CR0.PG.
 *  - Long mode, then `kernel_main`.
 */

/* XEN_ELFNOTE_PHYS32_ENTRY = 18 — 32-bit entry physical address. */
.section .note.Xen, "a", @note
.align 4
    .long 2f - 1f                 /* namesz */
    .long 4f - 3f                 /* descsz */
    .long 18                      /* type: PHYS32_ENTRY */
1:  .asciz "Xen"
2:  .align 4
3:  .long _start                  /* 32-bit entry PA (= VMA under identity) */
4:  .align 4

.section .text.boot, "ax"
.code32
.global _start
.type _start, @function
_start:
    cli
    movl $boot_stack_top32, %esp
    cld

    /* Zero .bss (NOLOAD; freestanding image has no CRT). */
    movl $__bss_start, %edi
    movl $__bss_end, %ecx
    subl %edi, %ecx
    xorl %eax, %eax
    shrl $2, %ecx
    rep stosl

    /* Zero boot page tables (separate section — not part of .bss). */
    movl $boot_pml4, %edi
    xorl %eax, %eax
    movl $((4096 * 3) / 4), %ecx
    rep stosl

    lgdt gdt64_ptr

    /* Identity-map the low 1 GiB with 2 MiB pages (PML4→PDPT→PD). */
    movl $boot_pdpt, %eax
    orl $0x3, %eax                /* Present|RW */
    movl %eax, boot_pml4

    movl $boot_pd, %eax
    orl $0x3, %eax
    movl %eax, boot_pdpt

    movl $boot_pd, %edi
    xorl %ecx, %ecx
0:
    movl %ecx, %eax
    shll $21, %eax
    orl $0x83, %eax               /* Present|RW|PS (2 MiB) */
    movl %eax, (%edi, %ecx, 8)
    incl %ecx
    cmpl $512, %ecx
    jb 0b

    /* CR4.PAE */
    movl %cr4, %eax
    orl $(1 << 5), %eax
    movl %eax, %cr4

    movl $boot_pml4, %eax
    movl %eax, %cr3

    /* EFER.LME */
    movl $0xC0000080, %ecx
    rdmsr
    orl $(1 << 8), %eax
    wrmsr

    /* CR0.PG (PE already set under PVH protected mode). */
    movl %cr0, %eax
    orl $(1 << 31), %eax
    movl %eax, %cr0

    ljmp $0x08, $long_mode_entry

.code64
long_mode_entry:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs

    leaq __stack_top(%rip), %rsp
    call kernel_main
1:
    hlt
    jmp 1b
.size _start, . - _start

/* Early 32-bit stack (pre long mode). */
.section .bss
.align 16
boot_stack32:
    .skip 0x4000
boot_stack_top32:

/* Boot page tables: dedicated NOLOAD section (progressive-isa B.7 / P.7). */
.section .boot.pt, "aw", @nobits
.align 4096
boot_pml4:
    .skip 4096
boot_pdpt:
    .skip 4096
boot_pd:
    .skip 4096

.section .rodata
.align 16
gdt64:
    .quad 0
    .quad 0x00af9a000000ffff      /* 64-bit code */
    .quad 0x00cf92000000ffff      /* data */
gdt64_end:
gdt64_ptr:
    .word gdt64_end - gdt64 - 1
    .long gdt64
