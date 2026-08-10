---
id: 0048
title: K8 design — SMP runqueue / IRQ model (first code in ADR-0070)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
amended: 2026-08-10
related: [0006, 0026, 0070, 0074]
---

# ADR-0048: SMP design (K8 — design; first code in ADR-0070)

## Acceptance status

**Accepted as design** (2026-08-08). First **code** slice landed in
[ADR-0070](0070-k8-smp-first-slice.md): unpark core 1 into idle only — **done
(HW)** (`smp: core1 alive`; Pi stamp 2026-08-09). No multi-core runqueue in this design ADR;
product path still schedules on one core until queue work.

## Decision (design)

| Item         | Intent                                                        |
| ------------ | ------------------------------------------------------------- |
| Topology     | Start with 2 cores on Pi 4B; rest parked                      |
| Runqueue     | Per-core ready queues + work stealing later                   |
| IRQ affinity | Timer per-core; GIC SPI affinity for devices                  |
| Sync         | IPI for remote wake; no IRQ-side switch (until K4 preemption) |
| Evidence     | QEMU `-smp 2` bring-up; HW dual-core stamp                    |

### First implementation slice

1. Unpark core 1 into idle WFI loop. — **paid (HW)** via ADR-0070
2. IPI wake (SGI 0 → core 1). — **paid (QEMU)** via ADR-0074; HW stamp residual
3. Per-core `current` + runqueues. — residual
4. Oracle: `smp: core1 alive`. — **paid (HW)** via ADR-0070
5. Oracle: `smp: core1 ipi`. — **paid (QEMU)** via ADR-0074

### Non-goals of this document

Full cache-coherent driver model; per-core runqueue (deferred to a later K8
slice). First unpark/idle is ADR-0070; IPI wake is ADR-0074.

## Deferral (historical)

Lab product path remains single schedulable core. Design originally waited on
dual-core gate investment after K4/K7; K4/K7 first slices are paid (HW), and the
unpark/idle gate is paid on QEMU and on silicon (ADR-0070; Pi stamp
2026-08-09). IPI wake is paid on QEMU (ADR-0074). Residual: **per-core
queues / current** depth (+ HW stamp for IPI).

> **Amendment (2026-08-09).** First unpark/idle slice implemented in
> [ADR-0070](0070-k8-smp-first-slice.md) (**done (QEMU)**). Steps 1 and 3 above
> are paid on QEMU; residual is HW stamp + per-core queues/IPI. “Code deferred”
> is no longer the live claim for the unpark path.

> **Amendment (2026-08-09, later same day).** HW stamp paid: transcript
> `20260809-160348.log` (`smp: core1 alive` on Cortex-A72). Silicon required
> PoC-clean of the spin-table words and the `SECONDARY_ROOT_PHYS` handoff (see
> ADR-0070 amendment). Residual is now per-core queues / IPI only.
> Reconciled by Claude on delegation from Gianluca.

> **Amendment (2026-08-10).** IPI wake paid on QEMU via
> [ADR-0074](0074-k8-ipi-wake-second-slice.md) (`smp: core1 ipi`). Residual is
> per-core queues / current and the IPI HW stamp.
