# Native multi-arch support — practices

**Status:** design contract (checklist and bar, not completion evidence).  
**Related:** [ADR-0015](../adr/0015-multi-arch-scaffold.md) (scaffold),
[ADR-0067](../adr/0067-host-lab-second-isa-intent.md) (lab x86 intent),
[`arch-contract.md`](../arch-contract.md), [`porting.md`](../porting.md),
[`host-lab-platform-matrix.md`](host-lab-platform-matrix.md).

This page is the **SSOT for how Harbor does native multi-arch support** and how
it stays **independent of the Linux stack** on product and guest paths. It does
not claim a second ISA is implemented.

**North star:** host-class native Harbor as primary OS on personal hardware
(L0 QEMU → L4 primary) is recorded in
[ADR-0069](../adr/0069-harbor-host-class-north-star.md). These practices apply
to every step of that path; QEMU lab is not the ceiling.

---

## Definitions

| Term | Means here | Does not mean |
| --- | --- | --- |
| **Native support** | Bare-metal Harbor on an ISA×board combo: real paging, traps, timer, console (or a faithful QEMU machine) | A `std` process on a host OS emulating traps |
| **Lab target** | Secondary combo for development/CI gates when a boot oracle exists | Product identity (that stays Pi 4B until a successor to ADR-0007) |
| **Host-class path** | L0–L4 maturity toward bare-metal Harbor on the laptop as primary OS ([ADR-0069](../adr/0069-harbor-host-class-north-star.md)) | “Ready to replace Arch” without an L3 workload name and stamp |
| **Product combo** | Officially marketed board+ISA with HW (or product-level) evidence | A green compile |
| **Supported** | Boot gate green for that combo; Makefile/CI allowlist open | Type-check only |
| **`done (QEMU-x86)`** | Lab x86 guest oracle under `qemu-system-x86_64` | `done (HW)` on Pi, or AArch64 `done (QEMU)` |
| **Dev host** | Machine that runs `cargo` / `make` / QEMU | Trusted computing base of the product image |
| **Linux-free guest** | No Linux ABI, bootloader chain, drivers, or userland in the target image path | “Must not use a Linux machine to build” |

---

## Linux-independence (planes A / B / C)

Do not collapse these three:

```text
A. Runtime / product     — Harbor on the target (Pi or QEMU guest)
B. Boot & port design    — how we enter and abstract hardware
C. Dev host / CI         — where humans and gates run tools
```

| Plane | Independent of Linux today? | Project bar |
| --- | --- | --- |
| **A — Runtime** | **Yes** (no Linux, no POSIX, no libc in the image) | **Must stay Linux-free** |
| **B — Boot/port** | **Yes if we choose** | **Must stay Linux-free** for lab and product paths |
| **C — Dev host** | **No in practice** (lab often Arch Linux) | **Allowed; non-TCB** — never import host OS assumptions into product code |

**Bar (normative):**

> Product and guest paths are Linux-free.  
> Dev host and the QEMU runner are allowed and named non-TCB.

### Family 0 — practices

| ID | Practice | Concrete rule |
| -- | -------- | ------------- |
| 0.1 | No Linux ABI or service in the guest | No Linux syscalls, vDSO, glibc, or Linux `init` |
| 0.2 | No Linux bootloader required | Lab x86 first slice: **`qemu-system-x86_64 -kernel <ELF>`** + `-serial stdio`. Multiboot2 only if an impl ADR needs memory map info (protocol, not “use GRUB”). No GRUB disk image, no Linux EFI stub |
| 0.3 | No mandatory Device Tree on x86 lab | Compiled BSP memmap (ADR-0011 spirit); DT is not a Linux dependency to reintroduce |
| 0.4 | No Linux drivers in-tree | Write protocols from specs; 16550/APIC/… are Harbor drivers |
| 0.5 | Linux only as prose analogy | ADR comments may cite Linux; code must not import Linux structure names as API |
| 0.6 | Harbor layering, not `linux/arch` clone | No `machine_desc`, softirq hierarchy, `altinstructions` model |
| 0.7 | Image names are firmware/Harbor-specific | Pi: `kernel8.img`. Lab x86: freestanding ELF with a Harbor name — not `vmlinuz` / `bzImage` |
| 0.8 | Host-test triple ≠ product target | `x86_64-unknown-linux-gnu` is the **harness** for `kernel-core` only |
| 0.9 | QEMU is a runner, not “the Linux stack” | Bare guest under QEMU is fine; do not require virtio-linux or guest-agent |
| 0.10 | Virtio only with a dedicated ADR | Disk/net lab devices are not smuggled semi-Linux |
| 0.11 | Freestanding toolchain for images | `*-unknown-none*` (or project equivalent); FP/SIMD policy is Harbor’s, not Linux’s |
| 0.12 | Document plane C as non-TCB | `stack.md` and this page: build host may be Linux; product image has zero Linux |

### Boot path preference (lab x86)

| Option | Verdict |
| ------ | ------- |
| QEMU `-kernel` ELF 64-bit | **Preferred** (0.2) |
| Multiboot2 via QEMU (no GRUB in CI) | Acceptable if justified in impl ADR |
| GRUB + disk image | Out of first slice; fails independence bar for lab workflow |
| UEFI + Linux EFI stub | Rejected for lab intent |
| Hosted `std` process | Rejected (ADR-0067) |

---

## Family 1 — Structure (ISA / board / drivers / policy)

| ID | Practice | Harbor enforcement |
| -- | -------- | ------------------ |
| 1.1 | Three planes: ISA, board bind, driver protocol | Architecture rules 1–3; ADR-0015 |
| 1.2 | Policy imports only `crate::arch` and `crate::bsp::board` | `make layering` |
| 1.3 | No `dyn Arch` for the whole CPU surface | ADR-0015 |
| 1.4 | Compile-time selection (`target_arch`, `board-*`) | `arch/mod.rs`, `bsp/mod.rs` |
| 1.5 | Explicit facade contract + gate vs re-exports | [`arch-contract.md`](../arch-contract.md) |
| 1.6 | Arch tree board-free (no hand-written board PAs) | `make arch-board-free` (extend if IO-port patterns appear) |
| 1.7 | Drivers take bases/IRQ ids from BSP only | Review + layering |
| 1.8 | Per-ISA boot, link, vectors under `arch/<isa>/` | ADR-0015 |

---

## Family 2 — “Supported” bar and levels

| Level | Minimum | Evidence label |
| ----- | ------- | -------------- |
| **Lab alive** | Boot → console TX → banner + `cpu:` + halt/idle | `done (QEMU-x86)` first claim |
| **Lab kernel** | + timer ticks + cooperative sched + exception path | deeper QEMU-x86 oracles |
| **Lab model** | + user session role + one agent-class oracle | model parity lab |
| **Product multi-arch** | Successor to ADR-0007 + product evidence policy | not implied by lab |

Rules:

- Supported ⇔ **boot gate green**, not compile-only (porting, ADR-0015, ADR-0067).
- Open Makefile `ARCH`/`BOARD` and CI **only** when that gate exists.
- Never collapse `done (QEMU-x86)` into `done (HW)` or AArch64 `done (QEMU)`.

---

## Family 3 — Pure core vs unsafe edge

| ID | Practice |
| -- | -------- |
| 3.1 | Authority, sched models, decode maths live in `kernel-core` (host-tested, Miri) |
| 3.2 | Platform identity decode is pure; arch only reads registers (ADR-0065 pattern; CPUID for x86) |
| 3.3 | `unsafe` confined to arch/drivers/bootstrap with `SAFETY` comments |
| 3.4 | A port must not weaken IPC/sched model tests to “make the port pass” |

---

## Family 4 — Evidence discipline

| ID | Practice | Harbor reference |
| -- | -------- | ---------------- |
| 4.1 | Boot gate asserts observable lines | `qemu-boot-check`, future `qemu-x86-boot-check` |
| 4.2 | INDETERMINATE vs FAIL when the runner is starved or tools missing | verification boot-check CPU quota |
| 4.3 | No CI skeleton target | porting.md |
| 4.4 | Product identity separate from lab marketing | ADR-0007, ADR-0067 |
| 4.5 | HW stamps remain Pi serial transcripts | verification.md |

---

## Family 5 — Vertical slice order

Implement a second ISA in thin vertical slices, each with an oracle:

```text
V0  Toolchain + link + _start → kernel_main (halt)
V1  Console TX + banner + cpu: self-check     ← first “lab alive”
V2  Timer IRQ + time::ticks + idle
V3  Cooperative sched (yield oracle)
V4  Exception + one syscall path
V5  User session (el0 role) + fault policy
V6  Agent beacon / console endpoint subset
V7+ Device agents / depth — only with a named lab need
```

| ID | Practice |
| -- | -------- |
| 5.1 | One oracle per slice; no big-bang arch-contract PR |
| 5.2 | First gate ≤ banner + `cpu:` + halt (ADR-0067) |
| 5.3 | User-mode is not required for “lab alive” |
| 5.4 | Pi-only stays Pi-only (SD durable, SPI TFT, VideoCore blobs, RNG200) unless a later ADR says otherwise |
| 5.5 | Design ADR before each boundary choice (boot details, irqchip, SIMD) |

---

## Family 6 — Naming debt (AArch64-shaped facade)

| ID | Practice |
| -- | -------- |
| 6.1 | Keep role names stable; ISA-local names until a second consumer exists |
| 6.2 | Alias (`el0`, `switch_ttbr0`, …) only in the port PR that needs them — no prep rename |
| 6.3 | Document role→impl mapping in the platform matrix |
| 6.4 | Keep IRQ-save tokens opaque (`u64`) so policy never sees DAIF/RFLAGS |

---

## Family 7 — IRQ, timer, idle

| ID | Practice | Harbor |
| -- | -------- | ------ |
| 7.1 | Separate clocksource, irqchip, and policy | `arch::timer`, `IrqChip`, `time`, `irq` |
| 7.2 | IRQ handlers do not context-switch (until a K4 successor) | ADR-0006 / 0008 |
| 7.3 | Idle waits only when no ready work and no lost wake | architecture rule 8 |
| 7.4 | Single irqchip owner | `irq::init` |
| 7.5 | IRQ path must not console-TX | rule 6 (16550 same as PL011) |

---

## Family 8 — Memory attributes

| ID | Practice |
| -- | -------- |
| 8.1 | Explicit early map before rich Rust (ADR-0003 spirit) |
| 8.2 | Device / UC mapping for MMIO (and correct PIO discipline on x86) |
| 8.3 | Carry W^X and guard *policy*, not AArch64 bit recipes blindly |
| 8.4 | Pre-MMU atomic rules are ISA-specific (A72 exclusives ≠ x86) — document, do not copy comments |
| 8.5 | User AS switch preserves the session protocol; CR3 vs TTBR is arch detail |

---

## Family 9 — Toolchain and images

| ID | Practice |
| -- | -------- |
| 9.1 | Freestanding target per ISA for the kernel image |
| 9.2 | Per-target linker/`rustflags` when a second target exists (generalize `build.rs` then, not “prep”) |
| 9.3 | Pinned toolchain (`rust-toolchain.toml`) |
| 9.4 | Board image names follow firmware (Pi) or Harbor lab naming (x86 ELF) |
| 9.5 | Exactly one `board-*` feature per image |

---

## Family 10 — Security

| ID | Practice |
| -- | -------- |
| 10.1 | Same agent threat model on every ISA (untrusted user program) |
| 10.2 | Do not weaken the cap ABI or refusal taxonomy for the port |
| 10.3 | Name lab residuals (no IOMMU, limited irqchip, …) instead of implying HW parity |
| 10.4 | QEMU evidence is not a Pi stamp |

---

## Family 11 — Process and docs

| ID | Practice | Status |
| -- | -------- | ------ |
| 11.1 | Intent ADR before code | ADR-0067 accepted |
| 11.2 | Platform matrix role→impl | `host-lab-platform-matrix.md` |
| 11.3 | Operational checklist | `porting.md` |
| 11.4 | Lab path is a standing watch, not a silent K/P reorder | `roadmap.md` |
| 11.5 | ADR index / architecture artefact table / xrefs stay consistent | `make xrefs` |
| 11.6 | Implementation ADR before first binary (boot, irqchip, SIMD) | when coding starts |
| 11.7 | Glossary carries evidence and lab terms | this page + glossary |

---

## Host-class destination (beyond QEMU)

[ADR-0069](../adr/0069-harbor-host-class-north-star.md) levels:

| Level | Platform sketch | Still Linux-free (A/B)? |
| ----- | --------------- | ------------------------ |
| L0 | QEMU x86 guest (`-kernel` ELF) | Yes |
| L1 | Bare-metal lab laptop, console-class | Yes — Harbor-owned boot (UEFI protocol ok; no Linux EFI stub identity) |
| L2–L4 | Storage/input/display/net as **agents**; named daily workload; optional Linux recovery only | Yes on the Harbor path |

Do not treat L1+ as “first slice” of ADR-0067; open separate implementation ADRs
and boards when those levels start. Practices 0.x–12 still apply.

## Family 12 — Anti-patterns

| Do not | Why |
| ------ | --- |
| Empty `arch/<isa>` compile-only skeleton | Bitrots; dilutes CI |
| `dyn Arch` megatrait | Wrong cost model for bare metal |
| Mass rename of EL0/TTBR “in preparation” | No second consumer yet |
| Hosted Linux process sold as native port | Plane A violation; different threat model |
| GRUB/UEFI Linux desktop boot as first lab path | Plane B coupling (0.2) |
| Copy Linux `arch/x86` or in-tree Linux drivers | License, model, dependency |
| Virtio “to look like a Linux guest” without ADR | Semi-Linux stack |
| Kernel target `…-linux-gnu` | Confuses harness with product |
| Require non-Linux dev host as a goal | Plane C is not TCB; low ROI |
| Steal Pi H2 priority by default for the lab port | ADR-0026 completeness goal |
| Claim multi-arch in README before a boot gate | ADR-0007 / 0067 |

---

## Gate map (existing ↔ practices)

| Gate / artefact | Practices it backs |
| --------------- | ------------------ |
| `make layering` | 1.2, 1.3-ish facade isolation |
| `make arch-board-free` | 1.6 |
| `arch-contract` vs facade check | 1.5 |
| `make test` / Miri on `kernel-core` | 3.1, 0.8 |
| `make boot-check` (AArch64) | 4.1 pattern |
| `make no-early-exclusives` / pre-MMU | 8.4 (AArch64-specific) |
| `make xrefs` / doc-claims | 11.5 |
| Future `qemu-x86-boot-check` | 2, 4.1, 5.2, 0.2 |

---

## What stays Pi-only by default

Unless a later ADR expands the lab composition:

- VideoCore firmware blobs and `kernel8.img` boot chain  
- PL011 / GICv2 / RNG200 / BCM SPI / ILI9486 / EMMC2 durable media  
- Pi serial HW stamps as `done (HW)`  

The lab path reimplements **roles** (console, irqchip, timer), not Pi silicon.

---

## Contributor checklist (complete when all answerable)

1. Native vs hosted vs product?  
2. Which facade modules (contract)?  
3. Vertical slice order?  
4. When to open Makefile / CI / README?  
5. Anti-patterns?  
6. Which gates must stay green; which new gate?  
7. Evidence labels?  
8. What is Pi-only?  
9. Linux-free (A/B) vs dev host (C)?  
10. Preferred x86 boot path (`-kernel` ELF)?  

If a port PR cannot answer these, stop and update this page or an ADR first.
