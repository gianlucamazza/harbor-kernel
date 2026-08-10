---
id: 0071
title: H3 L0 — x86_64 QEMU first slice (boot, console, cpu identity)
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0002, 0007, 0015, 0065, 0067, 0069]
---

# ADR-0071: H3 L0 x86_64 QEMU first slice

## Acceptance status

**Accepted** (2026-08-09). Implements the first **code** slice of
[ADR-0067](0067-host-lab-second-isa-intent.md) / L0 of
[ADR-0069](0069-harbor-host-class-north-star.md).

## Decision

| Item | Choice |
| --- | --- |
| Target | `x86_64-unknown-none` |
| Machine | QEMU `q35`, `-cpu qemu64` (or host), `-serial stdio -display none` |
| Boot | **PVH ELF note** (`XEN_ELFNOTE_PHYS32_ENTRY`) + QEMU `-kernel` (Linux-free; no GRUB). Multiboot1 rejected (ELF64). Multiboot2 alone not sufficient on QEMU 11 for this path. |
| Image | `target/.../harbor-x86.elf` (not `kernel8.img`) |
| Board feature | `board-qemu-q35` (mutually exclusive with `board-rpi4`) |
| Console | 16550 COM1 **port I/O** `0x3F8`, polled TX only |
| CPU line | CPUID decode in `kernel_core` + thin `arch` readers (ADR-0065 pattern) |
| SIMD | **Allowed** on lab x86 images (QEMU/TCG default); no softfloat pin. Documented divergence from ADR-0002 (AArch64 product). |
| Product tree | AArch64 modules **not compiled** on `target_arch = "x86_64"`; thin `src/lab/` entry (Pi stack not stubbed in) — [topology](../design/project-topology.md), [progressive-isa](../design/progressive-isa-practices.md) P.3 |
| Facade | Full arch-contract re-export; L0 call graph uses `cpu`/`mmio` + boot.s; other roles refuse or panic if entered (no silent success) |
| Practices | [progressive-isa-practices.md](../design/progressive-isa-practices.md), [native-multiarch-practices.md](../design/native-multiarch-practices.md) |

### Oracle

```text
Harbor: hello (x86 lab)
cpu: …               # vendor/family or QEMU identity
x86-lab: alive
```

Gate: `make x86-boot-check` → `scripts/boot/qemu-x86-boot-check.sh`.

### Non-goals (this slice)

Full `bootstrap::run` product path; agents; EL0/ring3; APIC timer; SMP;
bare-metal laptop; Multiboot as GRUB disk workflow; claiming multi-product.

## Evidence

| Claim | Gate |
| --- | --- |
| L0 alive | `make x86-boot-check` → `scripts/boot/qemu-x86-boot-check.sh` |
| No Pi regression | `make boot-check` (aarch64) still green |
| Layering | `make layering` |

### Design notes (settled; not workarounds)

- **Loader protocol:** QEMU ELF64 `-kernel` implements **PVH**
  (`XEN_ELFNOTE_PHYS32_ENTRY`). Multiboot1 rejects ELF64; Multiboot2 alone is
  not the path QEMU 11 takes for this image. One protocol, not a dual header.
- **Code model:** `small` (default) at 1 MiB load. `code-model=kernel` is the
  high-half (−2 GiB) model — wrong VMA class, not an optional flag.
- **Boot tables:** live in `.boot.pt` (NOLOAD), zeroed then filled in asm;
  `.bss` is separate (progressive-isa P.7).
- **CPU identity:** arch reads CPUID; family/model decode is pure in
  `kernel_core::cpuid` (ADR-0065).

## Related

- [0067](0067-host-lab-second-isa-intent.md), [0069](0069-harbor-host-class-north-star.md)
- [native-multiarch-practices](../design/native-multiarch-practices.md)
