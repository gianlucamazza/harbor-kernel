# Host/lab platform matrix — Pi 4 vs QEMU x86

**Status:** design contract for the lab second-ISA path.
**Intent ADR:** [ADR-0067](../adr/0067-host-lab-second-isa-intent.md).
**Practices:** [native-multiarch-practices.md](native-multiarch-practices.md)
(Linux-independence + support bar).
**Scaffold:** [ADR-0015](../adr/0015-multi-arch-scaffold.md),
[`arch-contract.md`](../arch-contract.md), [`porting.md`](../porting.md).

This page maps **roles** (what policy already depends on) to **today’s Pi 4
implementation** and a **candidate QEMU x86 lab** implementation. It is not a
completion claim: nothing here is `done (QEMU-x86)` until a boot gate says so.

Working names (may refine in an implementation ADR):

| Combo | ISA | Board feature | Runner |
| --- | --- | --- | --- |
| Product (today) | `aarch64` | `board-rpi4` | Pi 4B + QEMU `raspi4b` |
| Lab (intent) | `x86_64` | `board-qemu-q35` | `qemu-system-x86_64` (q35 preferred) |

---

## Architecture facade (`crate::arch`)

| Role (contract) | Pi 4 (AArch64) today | Lab QEMU x86 (candidate) | Rename / alias notes |
| --- | --- | --- | --- |
| `cpu` — IRQ mask, idle, halt, barriers, identity readers | DAIF; `WFI`; MIDR / ID_AA64* `mrs` | RFLAGS.IF (or CLI/STI); `hlt`; CPUID leaves | IRQ save token can stay opaque `u64` |
| `timer` — clocksource deadline / tick IRQ | Generic Timer CNTP | LAPIC timer, HPET, or PIT — pick one for first IRQ slice | Same policy surface in `time` |
| `mmu` — activate map, map/unmap, root phys, user AS switch | TTBR0/1, TCR, MAIR; `switch_ttbr0` | CR3; page-table walk 4-level; PAT/UC for device | Name `switch_ttbr0` may alias to “switch user root” when second consumer lands |
| `cache` — I/D + TLB maintenance | `dc`/`ic`, `tlbi` | `invlpg`, WBINVD as needed; x86 I/D often coherent for text | Empty/no-op paths possible where x86 allows |
| `switch` — cooperative context switch | AAPCS64 `Context` | SysV AMD64 callee-saved set | Layout is pure ISA ABI |
| `exception` — vectors, trap frame, init | VBAR + `vectors.s` | IDT + stubs; IST optional later | Frame field names may differ; policy uses published session GPR window |
| `el0` — user session enter/resume/end | EL0/EL1, SPSR, `eret` | ring3/ring0, `sysret`/`iretq` | **Module name stays `el0` until port PR needs otherwise** (arch-contract debt) |
| `mmio` — volatile MMIO | `read_volatile` / `write_volatile` | Same; ensure maps are UC | API can match |
| `probe` — soft external-abort recovery | Data abort recovery path | #PF / MCE not 1:1 — may stub “always present” first | Soft-fail devices may be Pi-only |
| `bootinfo` — firmware handoff | DTB pointer from firmware | Multiboot2 info or QEMU fw_cfg / zeroed | Optional for first slice |
| Session pointer for vectors | `CURRENT_EL0` linker symbol | Same obligation: sched publishes; vector path loads without a caller | Type is ISA-local |

### Per-ISA artefacts

| Artefact | Pi 4 | Lab QEMU x86 (candidate) |
| --- | --- | --- |
| Boot entry | `src/arch/aarch64/boot.s` — EL2→EL1h, early MMU, BSS, stack | `src/arch/x86_64/boot.s` (or `.asm`) — long mode, early page tables, BSS, stack → `kernel_main` |
| Linker script | load `0x80000`, `kernel8.img` | Freestanding ELF for **QEMU `-kernel`** (preferred); **not** `kernel8.img`, not `vmlinuz`/`bzImage` |
| Exception vectors | VBAR table | IDT + stub ISRs |

**Boot path (ADR-0067 + [native practices](native-multiarch-practices.md) §0.2):**
prefer **`qemu-system-x86_64 -kernel <ELF> -serial stdio`**. Multiboot2 only if
an implementation ADR needs memory-map handoff. No GRUB disk image, no Linux
EFI stub. Guest path is Linux-free; the machine running QEMU is non-TCB.

---

## Board (`crate::bsp::board`)

| Surface | Pi 4 (`rpi4`) | Lab (`qemu-q35` candidate) |
| --- | --- | --- |
| `memmap` | BCM2711 low peripherals `0xFE00_0000`, GIC, UART, RAM window, user VA | Identity RAM size for QEMU default; COM1 `0x3F8` (PIO, not MMIO); LAPIC/IOAPIC bases if used |
| `console` | PL011 bind + GPIO pinmux | 16550 bind at COM1; no pinmux |
| `irq` | GICv2 + timer/UART SPI/PPI ids | PIC and/or IOAPIC line routing into `irq` |
| `rng` | RNG200 optional | Optional: virtio-rng or omit for first slices |
| `gpio` / `display` / `pm` | Pi-specific | Omit or stub; no TFT lab path required |

Policy continues to import only `crate::bsp::board`.

---

## Drivers (protocols)

| Role | Pi 4 | Lab QEMU x86 (candidate) | Shared? |
| --- | --- | --- | --- |
| UART console | `drivers/pl011` | New `drivers/uart16550` (or similar) | No — different silicon |
| Irqchip | `drivers/gicv2` | New APIC/IOAPIC or PIC driver implementing `IrqChip` | No |
| SoC RNG | `drivers/rng200` | None for first slice | — |
| SD / SPI / panel | EMMC2, BCM SPI, ILI9486 | Out of lab scope (ADR-0067 non-goals) | Pi product only |

Drivers still take bases and IRQ ids from the BSP; they never name the board
crate.

---

## Platform self-check (ADR-0065 pattern)

| Piece | Pi 4 | Lab x86 |
| --- | --- | --- |
| Pure decode | `kernel_core::cpuid` over MIDR / ID_AA64* | Extend with CPUID leaf decode (vendor, family/model, features) — host-tested |
| Thin readers | `arch::cpu` `mrs` | `arch::cpu` CPUID instruction wrappers |
| Boot line | `cpu: Cortex-A72 …` | `cpu: GenuineIntel …` (or QEMU TCG CPU name) — unknown ≠ silence |
| Hard refusals | 4K granule, EL0/EL1 A64, ASID width | Long mode + required paging features; refuse if not 64-bit capable |

---

## Policy and `kernel-core` (do not rewrite for the port)

| Area | Port impact |
| --- | --- |
| `kernel_core::{ipc,tasks,cap,reply,…}` | None — pure integers |
| `sched`, `ipc` policy glue | Needs `arch::switch` / session publish only |
| `agent` session loop | Needs `arch::el0` role; names may stay EL0-shaped |
| `bootstrap` demos / loader | Feature/`cfg` board paths where memmap differs; prefer board facade |
| Durable SD, VideoCore blobs, SPI HAT | **Pi-only** unless a later ADR says otherwise |

---

## Softfloat / SIMD (open)

| | AArch64 product | x86 lab |
| --- | --- | --- |
| Policy today | Softfloat target; FPEN off ([ADR-0002](../adr/0002-softfloat-kernel.md)); `make no-simd` | **Unset** — decide in a follow-on ADR or first implementation ADR before link |
| Risk | N/A | Accidental SSE in compiler output; or over-constraining the lab image |

Do not assume ADR-0002 applies unchanged to x86.

---

## Early boot exclusives (architecture rule 7)

| | AArch64 / Cortex-A72 | x86 lab |
| --- | --- | --- |
| Issue | LDXR/STXR on Device-nGnRnE pre-MMU hang | Not the same memory-attribute exclusive story |
| Gate today | `make no-early-exclusives` / pre-MMU path scripts | Revisit: keep “no RMW atomics before paging” as discipline if useful, but do not copy A72 comments blindly |

---

## Evidence vocabulary

| Claim | Means |
| --- | --- |
| `done (QEMU)` | Existing AArch64 `raspi4b` (and product inject) oracles |
| `done (QEMU-x86)` | Lab combo boot-check (and later depth oracles) — **distinct** |
| `done (HW)` | Pi 4B serial stamps only |

---

## First-slice acceptance sketch (for a future implementation ADR)

Minimal serial (or stdio) lines the boot-check would require:

1. Kernel banner / identity string (Harbor, lab combo)
2. `cpu: …` self-check line (CPUID decode)
3. A single “alive” / halt-ready marker

Optional immediately after: timer tick report, then cooperative yield oracle.
User-mode session and agents are **later** slices, not first-gate scope.

---

## What not to put in this matrix

- SMP host topology (see [ADR-0048](../adr/0048-k8-smp-design.md) for Pi)
- Network / storage product paths
- Hosted userspace execution mode
- A claim that the matrix is implemented

When the first code lands, update rows that were wrong; do not expand scope
silently past [ADR-0067](../adr/0067-host-lab-second-isa-intent.md) non-goals.
