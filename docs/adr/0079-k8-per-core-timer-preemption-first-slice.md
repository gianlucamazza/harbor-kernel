---
id: 0079
title: K8 fourth slice — per-core timer and EL1 preemption on CPU 1
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0006, 0008, 0048, 0051, 0064, 0068, 0070, 0074, 0075, 0076, 0077, 0078]
---

# ADR-0079: K8 fourth slice (per-core timer + EL1 preempt on CPU 1)

## Acceptance status

**Accepted** (2026-08-10). Implements the first **code** slice of
[ADR-0078](0078-k8-per-core-timer-preemption-design.md): each schedulable
CPU programs its own CNTP, secondary enables PPI 30, global ticks stay
CPU0-only, and `el1_preempt_pending` evaluates per-CPU mirrors on both
affinities. Status **done (QEMU)** and **done (HW)** — Pi stamp 2026-08-10,
transcript `.serial-log/20260810-132749.log` (`preempt-el1-cpu1: rotated` +
`spinner exited` + `cpu: Cortex-A72 r0p3` + `CNTFRQ=54000000`;
`hw-transcript-check` clean; build `src=385cccee`).

## Decision (what landed)

### 1. Per-CPU arch timer

- `DEADLINE[2]` + shared `INTERVAL_COUNTS` (primary `timer::init` publishes)
- `timer::init_secondary()` arms this core’s CNTP from the published interval
- `on_interrupt` / `accelerate_next_tick` index by affinity

### 2. Timekeeping vs quantum IRQ

`time::on_timer_irq`:

1. Always re-arms **this** core (`timer::on_interrupt`)
2. Only affinity 0 advances `TICKS` / `MISSED` and signals the timer wait cookie

Secondary path never double-counts global time.

### 3. BSP secondary

After banked GICC + SGI 0: `init_secondary` → enable PPI 30 → mark ready →
unmask → `harbor_secondary_sched`. Handler table remains the shared sealed
set (timer cookie already registered on primary).

### 4. Preemption fence lifted

`el1_preempt_pending` no longer returns 0 for `affinity != 0`. Uses
`SLICE_START[cpu]` / `CURRENT_IS_IDLE[cpu]` vs global `time::ticks()` and
`BUDGET_QUANTUM_TICKS` (same predicate as K4).

### 5. Evidence

| Line | Meaning |
| --- | --- |
| `preempt-el1-cpu1: workers spawned` | Watcher + peer + spinner admitted |
| `preempt-el1-cpu1: rotated` | Non-yielding spinner on home=1 lost the CPU (peer saw ≥2 heartbeat rounds) |
| `preempt-el1-cpu1: spinner exited` | Stop word observed; spinner left its loop |
| `preempt-el1-cpu1: peer gave up` / `watch timeout` | Fail the boot oracle |

Workers on CPU 1 use atomics only (no console TX — ADR-0070). A thin
watcher on CPU 0 prints the lines.

Primary K4 oracles (`preempt-el1:`, `preempt:`) unchanged.

Gate: `boot-check` / `hw-transcript-check`. `MAX_TASKS` 43 → 46 (watcher +
peer + spinner live across the ADR-0031 auto-reap spawn window).

**HW stamp (2026-08-10):** transcript `.serial-log/20260810-132749.log` —
`smp: core1 alive` + `ipi` + `ran` + `preempt-el1-cpu1: rotated` +
`spinner exited` on Cortex-A72; primary K4 lines also clean on the same boot.

### 6. Residuals (honest)

- EL0 agents / EL0 preemption with `home = 1` — **design** [ADR-0080](0080-k8-el0-on-cpu1-design.md); code residual  
- Work stealing  
- Cores 2–3  
- Dual-core global tick producers (explicit non-goal)  
- Optional local quantum metric if global tick grain is too coarse

## Related

- Design: [0078](0078-k8-per-core-timer-preemption-design.md)
- Queues / discipline: [0075](0075-k8-per-core-queues-design.md),
  [0076](0076-k8-per-core-queues-first-slice.md),
  [0077](0077-smp-shared-state-discipline.md)
- K4: [0064](0064-k4-el0-preemption-first-slice.md),
  [0068](0068-k4-el1-preemption-second-slice.md)
