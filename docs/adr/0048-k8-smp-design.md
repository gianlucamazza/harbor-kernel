---
id: 0048
title: K8 design — SMP runqueue / IRQ model (design only)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0006, 0026]
---

# ADR-0048: SMP design (K8 — design accepted, code deferred)

## Acceptance status

**Accepted as design** (2026-08-08). First **code** slice:
[ADR-0070](0070-k8-smp-first-slice.md) (unpark core 1, idle only, QEMU).
No multi-core runqueue in this design ADR.

## Decision (design)

| Item | Intent |
| --- | --- |
| Topology | Start with 2 cores on Pi 4B; rest parked |
| Runqueue | Per-core ready queues + work stealing later |
| IRQ affinity | Timer per-core; GIC SPI affinity for devices |
| Sync | IPI for remote wake; no IRQ-side switch (until K4 preemption) |
| Evidence | QEMU `-smp 2` bring-up; HW dual-core stamp |

### First implementation slice (future)

1. Unpark core 1 into idle WFI loop.  
2. Per-core `current` + IPI wake.  
3. Oracle: `smp: core1 alive`.

### Non-goals of this document

Implementing SMP now; full cache-coherent driver model.

## Deferral

Lab is single-core product path; K8 waits for dual-core gate investment after K4/K7 design use.
