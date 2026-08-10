---
id: 0074
title: K8 second slice — SGI IPI wake core 1 (no runqueue yet)
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0006, 0008, 0048, 0068, 0070, 0075]
amended: 2026-08-10
---

# ADR-0074: K8 second slice (IPI wake)

## Acceptance status

**Accepted** (2026-08-10). Implements the next residual of
[ADR-0048](0048-k8-smp-design.md) after [ADR-0070](0070-k8-smp-first-slice.md):
prove that core 1 can take a **software-generated interrupt** raised by the
primary, run the shared IRQ dispatch path, and signal completion. Still **no**
per-core runqueue, no work stealing, no task migration.

**Evidence:** **done (QEMU)**; **done (HW)** Pi stamp 2026-08-10, transcript
`.serial-log/20260810-130305.log` (`smp: core1 ipi` + `cpu: Cortex-A72 r0p3` +
`CNTFRQ=54000000`; `hw-transcript-check` clean).

## Context

ADR-0070 leaves core 1 in `WFE` with IRQs **masked** forever. ADR-0048's next
design step is "per-core `current` + IPI wake". Full per-core `current` without
a second runqueue is mostly bookkeeping; the **evidence gap** is cross-core
interrupt delivery. This slice pays the IPI half and deliberately leaves
queues for a later ADR.

Preemption (ADR-0064/0068) remains **core-0 only**: shared `STARTED` /
`CURRENT_IS_IDLE` must not drive a switch on a secondary that has no TCB.

## Decision

### 1. Wake interrupt — GICv2 SGI 0

| Item | Choice |
| --- | --- |
| Line | **SGI 0** (software-generated, banked per CPU) |
| Raise | `GICD_SGIR` with `TargetListFilter=0`, `CPUTargetList` bit 1 (CPU interface 1) |
| Encoding | pure `kernel_core::gic::sgir_word` (host-tested) |
| Enable | banked `ISENABLER` on **core 1 only** for SGI 0 (primary never needs it to send) |
| Group | banked SGI+PPI Group 0 on core 1 (same bare-metal path as primary, ADR-0004) |

Not used: mailbox SEV-only wake (already proven alive); PSCI; PPI timer on core 1.

### 2. Secondary path after alive

After `CORE1_ALIVE` (ADR-0070), core 1 does **not** spin forever with DAIF set:

1. Park on `WFE` until the primary publishes `SECONDARY_MAY_IRQ` (after GICD +
   handler registration + seal).
2. Program **this CPU's** GICC (`PMR`/`BPR`/`CTLR`) and banked Group-0 for
   SGI+PPI; enable SGI 0; publish `SECONDARY_IRQ_READY` (PoC-clean).
3. Unmask IRQs; idle in `WFI` (interrupt-driven, not SEV).
4. On SGI 0: shared `handle_cpu_irq` → registered handler → set `CORE1_IPI`.

The secondary idle body lives in the **board IRQ bind** (`harbor_secondary_idle`
`no_mangle` symbol). Arch `secondary_main` only brings up MMU/VBAR/alive, then
branches by symbol — the same seam `boot.s` already uses for EL2 entry. Arch
still does not import `bsp`/`drivers` (layering rule 3).

### 3. Primary sequence (bootstrap)

After `board::irq::init` + `irq::seal` (SGI handler registered):

1. If core 1 is alive: set `SECONDARY_MAY_IRQ`, `SEV`, spin for `SECONDARY_IRQ_READY`.
2. Write SGIR targeting CPU 1.
3. Spin for `CORE1_IPI`.
4. Print `smp: core1 ipi` or `smp: core1 ipi timeout`.

Order relative to primary `irq_enable`: the probe may run with the primary
still masked; the **target** must be unmasked. Timer/UART stay CPU0-targeted
SPIs (unchanged).

### 4. Preemption stays on affinity 0

`el1_preempt_pending` returns 0 when `cpu::affinity() != 0`. Core 1 may take
IRQs; it must never enter `el1_preempt_pivot` against a scheduler that is not
yet multi-current. [ADR-0075](0075-k8-per-core-queues-design.md) owns
per-core queues; **per-core preemption** (timer PPI + quantum on core 1)
stays a later ADR — this fence remains until that slice, not merely until
queues land.

> **Amendment (2026-08-10).** Cross-link to ADR-0075: SGI 0 is promoted there
> to permanent RESCHED; the affinity preemption fence is tied to per-core
> preemption, not to queues alone.

### 5. Evidence

| Line | Meaning |
| --- | --- |
| `smp: core1 alive` | Unchanged (ADR-0070) |
| `smp: core1 ipi` | Core 1 handled SGI 0 and set the flag |
| `smp: core1 ipi timeout` | Fail the boot oracle |

Gate: `boot-check` / `hw-transcript-check` via `scripts/lib/boot-oracle.sh`.
Handler count seal line becomes **3** (timer + UART + wake SGI).

### 6. Explicit non-goals (still residual)

- Per-core runqueue / `current` array — **design** in ADR-0075; code residual
- Work stealing — later than queues code
- IPI remote **resched of tasks** — design in ADR-0075 (SGI 0 as RESCHED); code residual
- Per-core timer / PPI on core 1
- Unparking cores 2–3
- Cross-core K4 preemption
- Cache-coherent driver model beyond shared identity map

## Consequences

### Positive

- Dual-core **interrupt** path proven on QEMU (and HW when stamped)
- Shared dispatch table + `IrqChip` claim/EOI on a secondary without a second
  scheduler
- Thin oracle; residual queues remain an honest open row

### Negative / residual

- Secondary still runs no tasks
- `affinity()` guard is a temporary fence, not multi-core preemption design
- Full ADR-0048 "per-core current + IPI" is only half-paid (IPI)

### Gates

| Reversal | Catch |
| --- | --- |
| No `smp: core1 ipi` | `boot-check` / `hw-transcript-check` |
| Seal still claims 2 handlers | oracle handler-count line |
| Secondary preemption switch | `affinity() != 0` → no pivot; review |

## Related

- [0048](0048-k8-smp-design.md) — SMP design; residual queues
- [0070](0070-k8-smp-first-slice.md) — unpark / alive
- [0008](0008-irq-handler-policy.md) — handlers never switch
- [0068](0068-k4-el1-preemption-second-slice.md) — still core-0 only here
