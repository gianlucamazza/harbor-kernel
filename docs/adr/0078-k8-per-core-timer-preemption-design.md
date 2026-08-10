---
id: 0078
title: K8 design — per-core timer and IRQ preemption on CPU 1
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0006, 0008, 0048, 0051, 0064, 0068, 0070, 0074, 0075, 0076, 0077]
---

# ADR-0078: Per-core timer + preemption on CPU 1 (design)

## Acceptance status

**Accepted as design** (2026-08-10). Excellence path after K8 dual-current
**done (HW)** ([ADR-0076](0076-k8-per-core-queues-first-slice.md)/[0077](0077-smp-shared-state-discipline.md)):
CPU 1 can run tasks but has **no quantum IRQ path**, so a non-yielding worker
on affinity 1 owns the core forever. This ADR is the design for closing that
gap. First **code**: [ADR-0079](0079-k8-per-core-timer-preemption-first-slice.md)
(**done (QEMU)**; HW stamp residual).

## Context

| Piece | Today |
| --- | --- |
| Dual-current + sticky home | **done (HW)** — `smp: core1 ran` |
| Shared-state locks | **done (HW)** — `IrqSpinLock` heap/sched |
| `SLICE_START[N]` / `CURRENT_IS_IDLE[N]` | Per-CPU storage (0077) |
| `el1_preempt_pending` | Returns 0 when `affinity() != 0` — **honest**: no timer path on CPU 1 |
| Arch timer (`CNTP_*`) | Global `DEADLINE` + `INTERVAL_COUNTS`; `timer::init` only on bootstrap (CPU 0) |
| Secondary GIC | Banked GICC + SGI 0 only; **PPI 30 not enabled** on CPU 1 |
| Global `time::TICKS` | Advanced only from `time::on_timer_irq` (today: primary’s PPI) |

K4 (0064/0068) is complete on the **primary**. Dual-core **fairness** is not.

## Decision

### 1. Split timekeeping from local quantum IRQs

| Concern | Owner | Rule |
| --- | --- | --- |
| Global monotonic `ticks()` / park timeouts / boot oracle phase | **CPU 0 only** | Only affinity 0’s timer handler advances `TICKS` / `MISSED` and signals the timer IRQ-wait cookie |
| Force preemption epilogue on a core | **Each schedulable CPU** | Each core programs **its** CNTP and takes **its** banked PPI 30 |

**Rationale:** if both cores ran the full `on_timer_irq` at 10 Hz, `TICKS` would
advance at ~20 Hz and every quantum/oracle depending on tick rate would lie.
Double-counting is worse than a local-only re-arm path.

Handler sketch (code ADR names symbols):

```text
on_timer_irq(cookie):
  missed = timer::on_interrupt()   // re-arm THIS core's CNTP (per-CPU deadline)
  if affinity() == 0:
      TICKS += missed + 1
      MISSED += missed
      irq::wait::signal(cookie)    // global timer waiters stay primary-owned
  // affinity 1: re-arm only; epilogue may preempt using global ticks()
```

Quantum comparison on both CPUs continues to use **global** `time::ticks()` vs
`SLICE_START[cpu]` (same `BUDGET_QUANTUM_TICKS`). CPU 0’s tick series remains
the clock; CPU 1’s PPI only ensures the epilogue **runs** while a spinner holds
the core.

### 2. Per-CPU arch timer state

| Item | Choice |
| --- | --- |
| `INTERVAL_COUNTS` | Shared, published once by primary `timer::init` (read-only after) |
| `DEADLINE` | **Per-CPU** `[AtomicU64; N_CPUS]` (or equivalent); each core’s `on_interrupt` updates only its index |
| `CNTP_CVAL` / `CNTP_CTL` | Per-core system registers — already banked by architecture |
| `timer::init(hz)` | Primary only programs interval + first deadline + ENABLE |
| `timer::init_secondary()` (name free) | On CPU 1 after banked GIC: load interval, set first local deadline from `physical_count()`, ENABLE; **no** global TICKS side effects |

Pure deadline math stays in `kernel_core::timer` (already host-tested).

### 3. Secondary IRQ surface (BSP)

After existing banked GICC + SGI 0 bring-up ([ADR-0074](0074-k8-ipi-wake-second-slice.md)):

1. `timer::init_secondary()` (or equivalent)  
2. Enable **PPI 30** on the banked `ISENABLER` (same line id as primary; banked enable)  
3. Do **not** re-register handlers — shared sealed table already has `time::on_timer_irq`  
4. IRQs remain unmasked in secondary idle as today  

Order relative to `CPU1_ONLINE` / first schedule: timer secondary init **before**
unmask (or before any worker that must be preemptible); same spirit as primary
(bind → seal → unmask).

### 4. Preemption path on CPU 1 — EL1 first

| Path | First code slice (0079) | Later |
| --- | --- | --- |
| EL1 same-EL epilogue (`el1_preempt_pending` / pivot) | **In scope** — drop the `affinity() != 0 → 0` fence; evaluate `should_set(SLICE_START[cpu], ticks(), …, CURRENT_IS_IDLE[cpu])` | — |
| EL0 lower-EL preemption on CPU 1 | **Out of scope** until EL0-on-CPU1 ADR | Needs live EL0 session publish policy on secondary |
| Device IRQ handlers switch | Still **forbidden** (ADR-0008) | — |
| SGI handler | Still flag-only (0075/0077) | — |

EL1-first mirrors K4 history (0064 EL0 then 0068 EL1 on primary was product
need; here EL0 is not on CPU 1 yet, so EL1-only is the honest first slice).

Idle never preempted into idle: existing `CURRENT_IS_IDLE[cpu]` + `should_set`
predicate; CPU 1 idle stays unpreemptable.

### 5. Locks and IRQ context (unchanged doctrine)

- Timer handler: re-arm + optional tick accounting; **no** `IrqSpinLock`, no
  `switch`  
- Switch only on epilogue / voluntary path under existing sched lock rules
  (0077)  
- `SLICE_START[cpu]` / `CURRENT_IS_IDLE[cpu]` already updated on every
  `switch_on` (0077)

### 6. Evidence (for the code ADR)

| Claim | Gate (sketch) |
| --- | --- |
| CPU 1 timer live | Optional line `timer: cpu1 armed` or implicit via preempt oracle |
| Non-yielding EL1 spinner on home=1 loses CPU | Oracle e.g. `preempt-el1-cpu1: rotated` + peer progress (exact strings in code ADR) |
| Global tick rate unchanged | Existing tick/oracle phase still matches primary-only timekeeping |
| No regression primary K4 | Existing `preempt:` / `preempt-el1:` lines stay |
| QEMU then HW | `boot-check`; Pi stamp with `hw-transcript-check` |

Host: any pure multi-deadline indexing tests if extracted.

### 7. First implementation slice (follow-on code ADR ~0079)

Ordered:

1. Per-CPU `DEADLINE` in `arch::timer` + `init_secondary`  
2. `time::on_timer_irq` affinity split (global ticks only on CPU 0)  
3. BSP secondary: enable PPI 30 after GICC; call `init_secondary`  
4. Lift `el1_preempt_pending` affinity fence; use per-CPU mirrors  
5. Oracle: thin/full spinner `spawn_on(1, …)` that never yields; peer or
   stop-word pattern; assert rotation on CPU 1  
6. Docs: status **done (QEMU)** then HW stamp  

### 8. Explicit non-goals (this design / first code)

- EL0 sessions / agents with `home = 1`  
- Work stealing  
- Cores 2–3  
- Dual-core global tick producers  
- Changing `BUDGET_QUANTUM_TICKS` policy  
- Per-core runqueue redesign (already paid)  
- Making secondary own timer-wait cookies or park timeouts  

## Alternatives considered

| Alternative | Why not first |
| --- | --- |
| Both cores advance `TICKS` | Double-rate time; breaks quantum and oracle phase |
| CPU 1 uses only SGI resched for “preemption” | Not quantum fairness; depends on remote donor |
| Local tick counter only for quantum (no global ticks on CPU1 compare) | Extra conversion; global ticks already correct if CPU0 runs |
| Full EL0+EL1 preempt on CPU1 in one ADR | EL0-on-CPU1 is a separate product boundary |

## Consequences

### Positive

- Dual-core schedule becomes **fair** under non-cooperating EL1 work  
- Timekeeping stays single-producer (honest)  
- Reuses K4 epilogue and 0077 per-CPU mirrors  

### Residual after first code

- EL0-on-CPU1 + EL0 preempt on secondary  
- Steal  
- Optional local quantum metric if global ticks granularity is too coarse  

### Gates (design-level)

| Reversal | Catch |
| --- | --- |
| Both cores increment `TICKS` | Tick-rate / oracle phase drift; review |
| Fence left in place with timer live | Spinner on CPU1 never rotates; missing oracle line |
| Handler takes sched lock | Deadlock under load; 0077 |

## Related

- [0051](0051-k4-irq-preemption-design.md), [0064](0064-k4-el0-preemption-first-slice.md), [0068](0068-k4-el1-preemption-second-slice.md)  
- [0075](0075-k8-per-core-queues-design.md), [0076](0076-k8-per-core-queues-first-slice.md), [0077](0077-smp-shared-state-discipline.md)  
- [0048](0048-k8-smp-design.md), [0008](0008-irq-handler-policy.md)  
