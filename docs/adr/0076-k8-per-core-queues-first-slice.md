---
id: 0076
title: K8 third slice — per-core queues, dual current, pinned CPU1 worker
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0006, 0008, 0048, 0070, 0074, 0075, 0077]
amended: 2026-08-10
---

# ADR-0076: K8 third slice (per-core queues first code)

## Acceptance status

**Accepted** (2026-08-10). Implements the first **code** slice of
[ADR-0075](0075-k8-per-core-queues-design.md): dual-current pure model, coarse
sched lock, CPU1 idle + hard-affinity spawn, SGI resched kick, oracle
`smp: core1 ran`. Status **done (QEMU)** and **done (HW)** — Pi stamp
2026-08-10, transcript `.serial-log/20260810-130305.log` (`smp: core1 ran` +
`alive` + `ipi` + `cpu: Cortex-A72 r0p3` + `CNTFRQ=54000000`;
`hw-transcript-check` clean). Shared-state cleanup: [ADR-0077](0077-smp-shared-state-discipline.md).

## Decision (what landed)

### 1. Pure model (`kernel_core::tasks`)

- `N_CPUS = 2`, per-CPU `RunQueue` + `current` + sticky `home`
- `admit_on(cpu)`, `switch_on(cpu, kind)`, `start_cpu1()` for non-queued idle1
- CPU-0 convenience wrappers preserve existing host tests and `model_sched`

### 2. Sched lock + dual idle

- `AtomicBool` spinlock taken only with local IRQs masked
- Lock released before `context_switch` (no IPI-handler lock; ADR-0075 §4)
- `publish_el0`, `SLICE_START`, `CURRENT_IS_IDLE` updated **only on CPU 0**
  until per-core EL0/preemption

### 3. Secondary path

- After banked GIC bring-up: `harbor_secondary_sched` waits `CPU1_ONLINE`,
  then runs voluntary schedule on affinity 1
- Pinned marker sets `CORE1_RAN` and exits (stack free is SMP-safe after
  [ADR-0077](0077-smp-shared-state-discipline.md))
- Secondary remains a permanent schedule idle (no quiet-park; ADR-0077)

### 4. Evidence

| Line | Meaning |
| --- | --- |
| `smp: core1 ran` | Primary observed the pinned worker (printed on CPU 0) |
| `smp: core1 ran timeout` | Fail the boot oracle |

Gate: `boot-check` / `hw-transcript-check`. `MAX_TASKS` 42 → 43 (idle1; marker exits).

**HW stamp (2026-08-10):** transcript `.serial-log/20260810-130305.log` —
`smp: core1 alive` + `smp: core1 ipi` + `smp: core1 ran` on Cortex-A72;
durable store and K4 preemption oracles also clean on the same boot.

### 5. Residuals after ADR-0077 cleanup

- Per-core preemption / timer PPI on core 1  
- Work stealing; lock refinement if measured  
- EL0 agents with home on CPU 1

## Related

- Design: [0075](0075-k8-per-core-queues-design.md)
- Prerequisites: [0070](0070-k8-smp-first-slice.md), [0074](0074-k8-ipi-wake-second-slice.md)
