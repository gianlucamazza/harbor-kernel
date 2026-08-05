---
id: 0013
title: Narrow device MMIO windows for driver agents (F26 / M6)
status: proposed
date: 2026-08-05
---

# ADR-0013: Narrow device MMIO windows (proposed)

## Acceptance status

**Proposed.** Records finding **F26** and the shape M6 must follow. **Do not
implement agent MMIO maps until this ADR is accepted** (possibly refined).
Kernel EL1 may keep large Device regions for bring-up; **capability-mediated
maps must not**.

## Context

The multi-role review (F26) noted that Harbor’s Device mappings are **16 MiB
class blankets** (plus GIC). That is acceptable while only EL1 kernel code
touches MMIO. For **M6 driver-as-agent**, an EL0 agent that receives “the UART
window” must not also receive neighbouring peripherals in the same blanket.

M6 done-when: PL011 RX path as EL0 agent; console still echoes; **killing that
agent leaves the kernel ticking** — implies the agent’s map is revocable and
**minimal**.

## Decision (proposed)

### 1. Named sub-windows in the BSP

`bsp/rpi4/memmap` (or equivalent) exports **per-device** `(base, size)` for at
least:

| Device | Purpose |
| ------ | ------- |
| PL011 UART0 | Console TX/RX |
| GICD / GICC | Only if an agent ever owns IRQ chip pieces (likely never for M6 v1) |
| SPI0 | Optional; only if an SPI agent exists |
| RNG200 | Optional |

Sizes are **register block** sized (rounded up to page for Stage-1 map), not
16 MiB.

### 2. Two layers of mapping

| Layer | Who | Window size |
| ----- | --- | ----------- |
| Kernel EL1 identity / Device map | Kernel only | May remain coarse (today’s F26) until a P-milestone tightens it |
| Agent / capability map | EL0 driver agent | **Only** the named sub-window(s) for that device |

M6 v1 grants the PL011 agent **only** the UART register page(s).

### 3. Revocation

Destroying the agent (or revoking the MMIO capability) **unmaps** those pages
from its AS and must not leave executable or writable aliases. Kernel keeps
its own mapping for panic/steal console paths as today.

### 4. What this ADR does not decide

| Concern | Where |
| ------- | ----- |
| Frame allocator for RAM pages | [ADR-0012](0012-frame-allocator-for-address-spaces.md) |
| IRQ delivery to agents | successor / M6 design |
| Tightening kernel EL1 Device blankets | optional P-milestone; not required for M6 v1 |

## Consequences

### Positive

- F26 has a recorded shape before M6 code.
- Kill-agent done-when is implementable (small map, clear unmap).
- BSP remains the source of base/size (ADR-0011 board constants).

### Costs

- memmap becomes more verbose (many named constants).
- Kernel coarse map vs agent fine map must not be confused in reviews.

### Gate that would catch a reversal

| Reversal | Signal |
| -------- | ------ |
| Agent receives 16 MiB Device | Review / map dump in bringup; multi-role |
| Agent keeps MMIO after kill | M6 done-when fails; kernel still ticks but agent map remains |
| Bases hard-coded in agent binary | Layering: agent gets caps, not `memmap` imports |

## Alternatives considered

| Alternative | Why not |
| ----------- | ------- |
| Keep 16 MiB agent maps “for simplicity” | Defeats isolation; F26 stands |
| Remap entire kernel Device tree to 4 KiB before M6 | Large risk; not needed for M6 v1 done-when |
| IOMMU | Wrong platform scope for Pi 4B lab |

## When to accept

- Immediately before the first PR that maps MMIO into an EL0 agent AS.
- After M5 EL0 + AS machinery exists (needs ADR-0012 implemented).

## Related

- [architecture.md](../architecture.md) — M6 done-when; F26
- [ADR-0011](0011-dtb-mapped-board-constants-risk-accept.md) — board constants
- [ADR-0012](0012-frame-allocator-for-address-spaces.md) — RAM frames (not MMIO)
- Multi-role 2026-08-04 — F26
