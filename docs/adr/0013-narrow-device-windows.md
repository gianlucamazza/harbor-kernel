---
id: 0013
title: Narrow device MMIO windows for driver agents (F26 / M6)
status: accepted
date: 2026-08-05
accepted: 2026-08-05
---

# ADR-0013: Narrow device MMIO windows (accepted)

## Acceptance status

**Accepted** (2026-08-05). Closes finding **F26** for M6 design. Binding for
any EL0 agent MMIO map. Kernel EL1 may keep large Device regions for bring-up;
**capability-mediated / agent maps must not**.

## Context

The multi-role review (F26) noted that Harbor’s Device mappings are **16 MiB
class blankets** (plus GIC). That is acceptable while only EL1 kernel code
touches MMIO. For **M6 driver-as-agent**, an EL0 agent that receives “the UART
window” must not also receive neighbouring peripherals in the same blanket.

M6 done-when: PL011 path usable from EL0 with a **minimal** map; console still
works on the kernel path; **killing that agent** unmaps the window and leaves
the kernel ticking.

## Decision

### 1. Named sub-windows in the BSP

`bsp/rpi4/memmap` exports **per-device** base and agent map size, at least:

| Symbol | Purpose |
| ------ | ------- |
| `UART0_BASE` + `UART0_REG_BYTES` | PL011 register page for agents |
| `USER_PL011_VA` | Fixed user VA for that page in agent AS |
| GICD/GICC, SPI0, RNG200 | Named when an agent needs them — not bulk peripherals |

Sizes are **page-rounded Stage-1 granules**, not 16 MiB.

### 2. Two layers of mapping

| Layer | Who | Window size |
| ----- | --- | ----------- |
| Kernel EL1 identity / Device map | Kernel only | May remain coarse (F26) until a separate P-milestone |
| Agent AS map | EL0 driver agent | **Only** the named sub-window(s) for that device |

M6 v1 grants the PL011 agent **only** the UART register page via
`AddressSpace::map_device_page`.

### 3. Revocation

Destroying the agent AS **frees intermediate tables and drops the MMIO leaf**.
No RAM frame was owned for the MMIO leaf (PA is device). Kernel keeps its own
Device mapping for panic/console.

### 4. What this ADR does not decide

| Concern | Where |
| ------- | ----- |
| Frame allocator for RAM pages | [ADR-0012](0012-frame-allocator-for-address-spaces.md) |
| IRQ delivery to agents | successor |
| Tightening kernel EL1 Device blankets | optional P-milestone |

## Consequences

### Positive

- F26 has a binding shape before agent MMIO code.
- Kill-agent is implementable (small map, destroy AS).
- BSP remains the source of base/size (ADR-0011).

### Costs

- memmap carries more named constants.
- Kernel coarse map vs agent fine map must stay distinct in reviews.

### Gate that would catch a reversal

| Reversal | Signal |
| -------- | ------ |
| Agent receives 16 MiB Device | Review / map dump; multi-role |
| Agent keeps MMIO after kill | M6 oracle `pl011-agent: killed ok` fails or leak |
| Bases hard-coded only in agent .text | Agent uses fixed **VA**; **PA** comes from BSP at map time |

## Alternatives considered

| Alternative | Why not |
| ----------- | ------- |
| Keep 16 MiB agent maps “for simplicity” | Defeats isolation; F26 stands |
| Remap entire kernel Device tree to 4 KiB before M6 | Large risk; not needed for M6 v1 |
| IOMMU | Wrong platform scope for Pi 4B lab |

## Implementation note

M6 v1 smoke (2026-08-05): scheduled `pl011-agent` maps `USER_PL011_VA` →
`UART0_BASE` (one page), EL0 loads `UART_FR`, `SVC #0`, AS destroy. Evidence:
[verification.md](../verification.md).

## Related

- [architecture.md](../architecture.md) — M6 done-when; F26
- [ADR-0011](0011-dtb-mapped-board-constants-risk-accept.md) — board constants
- [ADR-0012](0012-frame-allocator-for-address-spaces.md) — RAM frames (not MMIO)
- Multi-role 2026-08-04 — F26
