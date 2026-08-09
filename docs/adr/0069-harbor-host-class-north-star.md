---
id: 0069
title: Harbor host-class north star — native primary OS on host hardware
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0007, 0015, 0026, 0067]
---

# ADR-0069: Harbor host-class north star

## Acceptance status

**Accepted as long-term intent** (2026-08-09). Records the owner’s destination
for Harbor as a **native primary OS** on host-class hardware (the lab laptop
that today runs Arch Linux), without changing today’s product board or
shipping claims.

**No code** is required by this ADR. **No rename** of the project: the public
and product name remains **Harbor**; the kernel/repo remains
**`harbor-kernel`** ([ADR-0007](0007-project-identity-harbor-kernel.md)).

## Context

Harbor is a verified agent-based microkernel and product OS whose **mission**
is agents, grants, evidence, and finishing the OS under that model
([ADR-0026](0026-kernel-and-product-completeness.md), vision). Completeness
tracks (K/P) and horizons H0–H2 are oriented around the Raspberry Pi 4B
product path and the capability model.

Separately, multi-arch scaffold ([ADR-0015](0015-multi-arch-scaffold.md)),
lab x86 under QEMU ([ADR-0067](0067-host-lab-second-isa-intent.md)), and
[native multi-arch practices](../design/native-multiarch-practices.md)
(Linux-free guest paths) prepare a second ISA without claiming host-class
product support.

The owner’s long-term goal is stronger than “lab guest” or “Pi appliance”:

> One day Harbor should be **usable natively in place of the current OS**
> on the machine that is today’s development host.

That goal must be written down so it does not get confused with:

- Linux/POSIX compatibility or running unmodified Linux apps  
- A hosted kernel under Linux forever  
- Abandoning Pi completeness as the near-term model lab  
- Renaming the project  

## Decision

### 1. North star

Harbor’s long-term destination includes **host-class native use**: bare-metal
Harbor on personal/host hardware (initially the lab x86_64 laptop), enough
agents and compositions that a **named daily workload** can run **without** a
Linux kernel underneath for that work.

This is **replacement**, not emulation: Linux-free product/guest paths stay
normative ([native-multiarch-practices](../design/native-multiarch-practices.md)
planes A/B). Linux may remain only as **dev host** (plane C) or optional
**recovery** dual-boot until primary-OS use is declared.

### 2. Naming (unchanged)

| Surface | Name |
| --- | --- |
| Project / product OS / prose | **Harbor** |
| Repository / package / kernel binary identity | **`harbor-kernel`** |

Do **not** introduce a second product brand for host-class. “Harbor” already
names the protected place for bounded components — kernel and full OS alike.
Platform-specific image names (e.g. `kernel8.img` on Pi) stay firmware-local
([ADR-0007](0007-project-identity-harbor-kernel.md)).

### 3. Relationship to current identity and lab path

| Decision | Holds |
| --- | --- |
| Official product board **today** | Raspberry Pi 4 Model B (ADR-0007) until a **successor** ADR expands product platforms |
| Lab x86 QEMU ([ADR-0067](0067-host-lab-second-isa-intent.md)) | **L0** of the host-class path — not the ceiling; bare-metal laptop remains out of *first* slice but **on** the path |
| H0–H2 / K/P completeness on Pi | **Still primary** for proving the agent/cap model; host-class must not silently reorder Pi “next working order” without owner intent |
| Elevating laptop to product combo | Requires a successor to ADR-0007 (and evidence bar), not implied by this ADR or first QEMU boot |

### 4. Horizon H3 (narrative)

Vision gains horizon **H3 — Host-class native Harbor**:

- Native boot on host-class hardware  
- Agents sufficient for a named workload **in place of** the previous OS for that slice of life  
- Evidence vocabulary distinct from Pi `done (HW)`  

H3 is **open / intent**. Status of mechanisms remains in the K/P tables and
future host-class tracks — not a fake “done” here.

### 5. Maturity levels (honest bars)

Claims about “instead of the current OS” use levels, not a single boolean:

| Level | Meaning | Evidence sketch |
| ----- | ------- | ---------------- |
| **L0** | Lab guest under QEMU x86 | `done (QEMU-x86)` — ADR-0067 path |
| **L1** | Bare-metal bring-up on the lab laptop (console-class I/O, halt/idle) | `done (HW-x86)` bring-up stamp |
| **L2** | Self-host tools: storage + I/O enough to edit/build a named tool (e.g. Harbor itself) on Harbor | stamp + recipe |
| **L3** | **Daily slice**: owner-named workload runs without rebooting to Linux for that work | real use documented |
| **L4** | **Primary OS** for the declared life-slice; Linux optional recovery only | owner decision note |

**Owner goal = L3 → L4.**  
The **L3 workload name is TBD** until the owner fixes it; no L3 claim without
that name.

### 6. Non-goals of this north star

- Binary compatibility with Linux or POSIX parity  
- Shipping host-class support in the public README while product board is Pi-only  
- Dropping evidence/gates to rush bring-up  
- GRUB/Linux EFI stub as the long-term boot identity (Harbor-owned boot path; UEFI protocol as firmware interface is fine)  
- In-tree Linux drivers or cloning `linux/arch`  
- Renaming Harbor  

### 7. Path order (multi-year, non-calendar)

```text
Model completeness (Pi H0–H2 / K/P)     ──continues──►
Lab x86 L0 (QEMU)                       ──then──►
Bare-metal laptop L1                    ──then──►
Self-host L2 → daily L3 → primary L4
```

Implementation still follows [porting](../porting.md) and
[native-multiarch-practices](../design/native-multiarch-practices.md):
no empty ISA skeleton; boot gate before “supported.”

## Consequences

### Positive

- The destination is explicit: native primary OS, same Harbor model  
- Naming stays stable (ADR-0007)  
- Lab QEMU and Pi work are steps, not rival projects  
- Over-claiming is harder: L0–L4 force honesty  

### Negative / debt

- Host-class needs future boards (UEFI PC / laptop), drivers-as-agents (storage,
  input, display, net), and threat-model residuals beyond Pi serial  
- Tension between Pi next-work and host-class investment — resolved by owner
  priority, not by this ADR stealing roadmap order  
- L3 workload still unnamed  

### Gates that catch reversal

| Reversal | Catch |
| --- | --- |
| Marketing “Harbor replaces Linux” at L0 | No product claim without L3+ evidence; README stays Pi-first until ADR-0007 successor |
| Hosted-under-Linux sold as north star | Rejected here and in ADR-0067 |
| Second brand / rename | Naming section + ADR-0007 |
| Skipping boot gates for “daily use” | porting + practices + verification discipline |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| New product name (e.g. a separate OS brand) | Owner: real name remains Harbor |
| Host-class as immediate product board | Model and gates not ready; Pi remains official platform |
| Linux ABI for app compatibility | Contradicts vision and Linux-free bar |
| North star = QEMU only | Owner wants native replacement of the current OS |

## Related

- [0007](0007-project-identity-harbor-kernel.md) — Harbor / harbor-kernel identity  
- [0026](0026-kernel-and-product-completeness.md) — finish the OS under this model  
- [0015](0015-multi-arch-scaffold.md) — multi-arch ready  
- [0067](0067-host-lab-second-isa-intent.md) — QEMU x86 lab (L0)  
- [`vision.md`](../vision.md) — H3 narrative  
- [`design/native-multiarch-practices.md`](../design/native-multiarch-practices.md)  
