---
id: 0048
title: K8 design — SMP runqueue / IRQ model (first code in ADR-0070)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
amended: 2026-08-10
related: [0006, 0026, 0070, 0074, 0075]
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
2. IPI wake (SGI 0 → core 1). — **paid (HW)** via ADR-0074 (stamp 2026-08-10)
3. Per-core `current` + runqueues. — **design** [ADR-0075](0075-k8-per-core-queues-design.md); first **code** [ADR-0076](0076-k8-per-core-queues-first-slice.md)/[0077](0077-smp-shared-state-discipline.md) **done (HW)** (stamp 2026-08-10)
4. Oracle: `smp: core1 alive`. — **paid (HW)** via ADR-0070
5. Oracle: `smp: core1 ipi` / `smp: core1 ran`. — **paid (HW)** via ADR-0074/0076 (transcript `20260810-130305.log`)

### Non-goals of this document

Full cache-coherent driver model; work stealing (still later). First
unpark/idle is ADR-0070; IPI wake is ADR-0074; queue **design** is ADR-0075.

## Deferral (historical)

Lab product path remains single schedulable core. Design originally waited on
dual-core gate investment after K4/K7; K4/K7 first slices are paid (HW), and the
unpark/idle gate is paid on QEMU and on silicon (ADR-0070; Pi stamp
2026-08-09). IPI + queues first + shared-state are paid on silicon
(ADR-0074/0076/0077; stamp 2026-08-10, transcript `20260810-130305.log`).
Residual (live): **EL0-on-CPU1** (design [ADR-0080](0080-k8-el0-on-cpu1-design.md))
and **steal**. Per-core EL1 preempt **done (HW)** via [ADR-0079](0079-k8-per-core-timer-preemption-first-slice.md).

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

> **Amendment (2026-08-10, later).** Queue / multi-current **design** accepted
> in [ADR-0075](0075-k8-per-core-queues-design.md). Code residual + steal +
> per-core preemption remain; IPI HW stamp still open.

> **Amendment (2026-08-10, queues code).** First queues code paid on QEMU via
> [ADR-0076](0076-k8-per-core-queues-first-slice.md) (`smp: core1 ran`). Residual:
> steal, per-core preemption, multi-core heap, HW stamps.

> **Amendment (2026-08-10, HW stamp).** IPI + queues first + shared-state
> discipline paid on silicon: transcript `20260810-130305.log` (`smp: core1
> alive` + `ipi` + `ran`, Cortex-A72, `hw-transcript-check` clean). Residual:
> steal, per-core preemption, EL0-on-CPU1.
