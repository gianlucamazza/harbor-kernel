---
id: 0067
title: Host/lab second ISA — QEMU x86_64 intent and non-goals
status: accepted
date: 2026-08-09
accepted: 2026-08-09
amended: 2026-08-09
related: [0002, 0007, 0015, 0026, 0065]
---

# ADR-0067: Host/lab second ISA (intent — QEMU x86 bare guest)

## Acceptance status

**Accepted as design intent** (2026-08-09). Records *why* and *what* a second
ISA is for Harbor, and what it is not. **No `src/arch/x86_64/` (or board
package) lands under this ADR alone** — implementation follows
[`porting.md`](../porting.md) only when a boot gate exists.

Product identity remains Raspberry Pi 4 Model B
([ADR-0007](0007-project-identity-harbor-kernel.md)). This ADR does **not**
make Harbor multi-product.

## Context

Harbor is multi-arch *ready* ([ADR-0015](0015-multi-arch-scaffold.md)):
`crate::arch` is `target_arch`-selected, boards are `board-*` features, and
policy must not import ISA or board paths. The product combo is still only
AArch64 + `board-rpi4`.

“Host” in the tree today means three *other* things:

| Surface | Meaning today |
| --- | --- |
| Host tests | `kernel-core` on `x86_64-unknown-linux-gnu` — pure logic, no kernel image |
| `scripts/host/` | Lab tooling (SD, serial, mutants) on the development machine |
| QEMU CI/lab | Guest still AArch64 (`raspi4b`), not an x86 kernel |

None of those is “Harbor boots on the laptop.” Supporting the lab laptop as a
**runner for a second guest ISA** is a new product *path*, not an extension of
host tests. Without an intent ADR, the tree either grows empty stubs (forbidden
by ADR-0015 / porting) or renames AArch64-shaped APIs “in preparation” (also
forbidden until a second consumer exists —
[`arch-contract.md`](../arch-contract.md) § Known debt).

## Decision

### 1. Mode: QEMU x86 bare guest (not bare-metal PC, not hosted process)

| Choice | Detail |
| --- | --- |
| **Execution** | Harbor as a **bare-metal guest** under `qemu-system-x86_64` |
| **Machine (intent)** | Prefer **q35**; `virt` is acceptable if it shortens the first slice |
| **Boot path (intent)** | Prefer **QEMU `-kernel` freestanding ELF** + `-serial stdio` — no GRUB, no Linux EFI stub, no disk image bootloader chain ([native practices](../design/native-multiarch-practices.md) §0.2). Multiboot2 only if a later impl ADR needs it for memory-map handoff |
| **ISA** | `x86_64` — new `src/arch/x86_64/` when implemented |
| **Board** | New `board-*` (working name **`qemu-q35`**) — **compiled** memmap, COM1, IRQ line ids (no mandatory DT) |
| **Host laptop role** | Runs the emulator only (e.g. Tiger Lake lab machine). It is **not** a Harbor BSP |
| **Linux stack** | Guest and boot path are **Linux-free** (plane A/B). Dev host OS and QEMU runner are non-TCB (plane C) — see [native-multiarch-practices](../design/native-multiarch-practices.md) |

**Rejected for this intent (or separate ADR if ever wanted):**

| Mode | Why not here |
| --- | --- |
| Bare-metal boot on the physical laptop | UEFI/ACPI/PCIe/GPU — wrong order of magnitude for validating agents/caps |
| Hosted userspace kernel (process on Linux) | Different threat model and evidence claim; not a second ISA under ADR-0015 |
| Second AArch64 board only | Does not exercise the multi-arch facade with a non-AArch64 consumer |
| GRUB / Linux EFI stub / `vmlinuz`-shaped images | Couples lab boot to desktop Linux stack; fails independence bar |
| In-tree Linux drivers or `linux/arch` clone | Wrong model, license surface, and dependency graph |

### 2. Identity: lab target secondary to Pi 4

| Surface | Rule |
| --- | --- |
| Product board | Remains Pi 4B (ADR-0007). README/stack marketing stay Pi-first |
| Lab target | x86_64 + QEMU board is **development / CI lab** when a boot gate exists |
| Evidence label | Prefer **`done (QEMU-x86)`** (or equivalent) — never collapse into `done (HW)` Pi stamps |
| Elevating to product combo | Requires a **successor** to this ADR and ADR-0007 — not implied by first boot |
| Host-class north star | Lab QEMU is **L0** on the path to native Harbor on the laptop as primary OS ([ADR-0069](0069-harbor-host-class-north-star.md)); bare-metal laptop is out of *this* first slice, not out of the project |

### 3. “Supported” means boot gate, not compile

Aligned with [`porting.md`](../porting.md):

1. No empty ISA skeleton that only type-checks.
2. Makefile `ARCH` / `BOARD` allowlist opens **only** when an equivalent of
   `boot-check` exists for that combo.
3. CI job for the combo only when that gate is green.
4. Facade isolation and layering gates apply unchanged (`make layering`).

### 4. What stays invariant

The port exists to run the **same Harbor model** on another ISA:

- agent = kernel driver task + user program (session), not a POSIX process
- authority = slot-indexed capabilities; raw caps never leave the kernel
- cooperative policy layer; IRQ handlers do not context-switch (until K4
  successors say otherwise — those tracks remain Pi-shaped unless ported)
- evidence ≠ compile: oracles assert serial (or equivalent) lines
- `kernel-core` remains pure and host-tested; the port does not rewrite it

Layering: **ISA** implements [`arch-contract.md`](../arch-contract.md);
**board** binds memmap/console/irq; **drivers** implement protocols (16550,
APIC/IOAPIC or PIC, timer) — not GIC/PL011 reuse unless silicon matches.

### 5. Naming and Makefile (when code lands)

| Item | Intent name (may refine in implementation ADR) |
| --- | --- |
| `ARCH` | `x86_64` |
| Cargo target | `x86_64-unknown-none` (or project-chosen equivalent with softfloat/SIMD policy settled) |
| `BOARD` / feature | `qemu-q35` / `board-qemu-q35` |
| Image | Not `kernel8.img` — that name is Pi firmware-specific (ADR-0007) |

AArch64-shaped facade names (`el0`, `switch_ttbr0`, …) stay until the second
consumer **forces** alias or rename — no prep rename commit.

### 6. First implementation slice (future code — acceptance sketch)

Not implemented by this ADR. When opened, the vertical slice is deliberately
thin (foundation M0 spirit), **Linux-free boot**:

1. Freestanding ELF entered via QEMU `-kernel` → minimal map → `kernel_main`
2. Polled console TX (16550 COM1)
3. Banner + `cpu:` line via CPUID decode (extend `kernel_core::cpuid` pattern
   from [ADR-0065](0065-platform-self-check.md))
4. Halt / idle
5. `scripts/boot/qemu-x86-boot-check.sh` (or equivalent) asserting a few lines

Then, separate slices: timer ticks, cooperative sched, syscall/user session,
beacon-class agent. Pi-only media (SD durable, SPI display, VideoCore blobs)
stay Pi-only unless a named composition needs them on the lab target.

### 7. Explicit non-goals of this intent

- Claiming multi-arch product support in the public README
- SMP on the lab target (K8 remains Pi design — [ADR-0048](0048-k8-smp-design.md))
- Network / durable storage / product display on the lab target (P3/P4/P2 media)
- `dyn Arch` / runtime ISA switch
- Softfloat/SIMD policy on x86 — **open**; either a follow-on ADR or an explicit
  “allow SSE in lab images” decision before the first binary lands
  ([ADR-0002](0002-softfloat-kernel.md) is AArch64 product history)
- Reordering H2 Pi completeness (K4 same-EL, K8) behind this lab port by default

## Platform matrix and native practices

Role-by-role mapping Pi → QEMU-x86 lives in
[`docs/design/host-lab-platform-matrix.md`](../design/host-lab-platform-matrix.md).
Native multi-arch + Linux-independence checklist:
[`docs/design/native-multiarch-practices.md`](../design/native-multiarch-practices.md).
Both are design contract, not completion evidence.

### Amendment (2026-08-09)

Reconciliation: preferred boot path fixed to **QEMU `-kernel` ELF** and the
Linux-independence bar (planes A/B/C) cross-linked to the practices doc — no
change to mode (QEMU x86 bare guest) or product identity (Pi 4B).

## Consequences

### Positive

- Second ISA work has a named scope, evidence vocabulary, and refuse list.
- Scaffold (ADR-0015) is exercised with a real consumer plan, not a stub.
- Pi product identity and K/P completeness tracking stay primary.

### Negative / debt

- Softfloat/SIMD and early-boot exclusives rules are ISA-specific; x86 needs
  its own written policy before code.
- Facade names remain AArch64-shaped until the port PR needs them not to.
- Two boot oracles to maintain once the gate exists.

### Gates that catch reversal

| Reversal | Gate |
| --- | --- |
| Empty `arch/x86_64` without boot-check | Review + this ADR; porting checklist |
| Policy imports `arch::x86_64` / `bsp::qemu_q35` | `make layering` (once modules exist) |
| Claiming `done (HW)` for QEMU-x86 | Evidence vocabulary in this ADR + verification discipline |
| Marketing multi-arch without runner | README/stack ownership; ADR-0007 until successor |
| Hosted process sold as this port | Explicit reject table above |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Compile-only x86 skeleton now | ADR-0015 / porting: bitrots, dilutes CI |
| Hosted Linux process as “the” host support | Different architecture; separate ADR if wanted |
| Bare-metal laptop as first slice | Scope explosion unrelated to agent/cap proof |
| Rename EL0/TTBR everywhere first | Zero second consumer; churn without payoff |
| Treat as K-track peer to K4/K8 | Lab path; must not silently steal Pi completeness order |

## Related

- [0015](0015-multi-arch-scaffold.md) — multi-arch scaffold
- [0007](0007-project-identity-harbor-kernel.md) — product identity Pi 4
- [0002](0002-softfloat-kernel.md) — softfloat (AArch64 product)
- [0065](0065-platform-self-check.md) — CPU identity at boot
- [0026](0026-kernel-and-product-completeness.md) — completeness goal (Pi K/P)
- [`porting.md`](../porting.md), [`arch-contract.md`](../arch-contract.md)
- [`design/host-lab-platform-matrix.md`](../design/host-lab-platform-matrix.md)
- [`design/native-multiarch-practices.md`](../design/native-multiarch-practices.md)
- [0069](0069-harbor-host-class-north-star.md) — host-class native Harbor (L0–L4); this ADR is L0 only
