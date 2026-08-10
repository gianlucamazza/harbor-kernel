---
id: 0075
title: K8 design — per-core ready queues and current (code in follow-on)
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0006, 0008, 0048, 0051, 0064, 0068, 0070, 0074]
---

# ADR-0075: K8 per-core queues (design)

## Acceptance status

**Accepted as design** (2026-08-10). First **code** slice:
[ADR-0076](0076-k8-per-core-queues-first-slice.md) (**done (QEMU)** and
**done (HW)**, stamp 2026-08-10, transcript `20260810-130305.log`); shared-state
cleanup [ADR-0077](0077-smp-shared-state-discipline.md).

Parent design intent remains [ADR-0048](0048-k8-smp-design.md). Unpark and
IPI delivery are paid by [ADR-0070](0070-k8-smp-first-slice.md) /
[ADR-0074](0074-k8-ipi-wake-second-slice.md). This ADR is the residual those
left open: **where ready work lives**, **who is current on each core**, and
**how a wake crosses cores** without inventing work stealing.

## Context

| Layer | Today | Why it blocks multi-core schedule |
| --- | --- | --- |
| `kernel_core::tasks` | One `current`, one `RunQueue` | Decision model is uniprocessor |
| `src/sched` | One `SyncCell<Sched>`; exclusion = IRQ mask on **one** core | Two cores with DAIF set do not exclude each other |
| Preemption (K4) | Shared `SLICE_START` / `CURRENT_IS_IDLE`; ADR-0074 fences `affinity() != 0` | Fence is “no preemption on core 1”, not per-core quantum |
| Core 1 | Alive + SGI 0 path (0070/0074) | Idle `WFI`, **zero** tasks |
| ADR-0048 | “Per-core queues + steal later” | Named residual, no mechanism |

Masking IRQs was a sound exclusive-access story only while a single core
ran policy. Landing queues without naming locking, affinity, idle-on-secondary,
and the IPI↔lock rule would freeze those choices in the first patch that
links — the failure mode ADR-0001 exists to prevent.

## Decision

### 1. Topology

| Item | Choice |
| --- | --- |
| Schedulable CPUs | **`N_SCHED_CPUS = 2`** (affinity 0 and 1) |
| Cores 2–3 | Stay parked (unchanged from ADR-0070) |
| Product claim | Still “one schedulable core” until the first **code** slice is evidence-green; this ADR does not rename H2 |

### 2. Per-core ready queues and multi-current

Ready work is **not** a single global FIFO. The pure model (host-tested in
`kernel_core`) gains a CPU axis:

```text
Tasks  (sketch — names free in the code ADR)
  states[N], epochs[N], parked, …     // one lifecycle per slot (global)
  home[N]: CpuId                      // sticky affinity (see §3)
  queue: [RunQueue; N_SCHED_CPUS]     // ready lists, local to a CPU
  current: [TaskId; N_SCHED_CPUS]     // Running occupant per CPU
```

Invariants (must be host-tested):

1. A task in `Ready` sits on **at most one** per-core queue.
2. `current[c]` is `Running` on CPU `c` (or the idle id for that CPU).
3. Idle ids are never enqueued.
4. `switch(cpu, kind)` consults only `queue[cpu]` for the next ready task.

`admit` records `home`. `wake(id)` enqueues onto `queue[home[id]]` and
returns whether a **remote** CPU must be kicked (home ≠ waker’s CPU).

If the pure API shape grows awkward, a dedicated `kernel_core` module is
allowed; the split stays “decide pure / act in `sched`”.

### 3. Affinity — hard sticky, no steal (first era)

| Operation | Rule |
| --- | --- |
| Default spawn | `home = 0` |
| Explicit spawn | `spawn_on(cpu, …)` for lab/oracle (and any future product need) |
| Migration | **None** in the first code slice and not implied by this design |
| Work stealing | **Out of scope** of this design — later [ADR-0082](0082-k8-work-stealing-design.md) |

A task always becomes Ready on its home queue. Soft affinity and load
hints are not part of this design.

### 4. Mutual exclusion — coarse sched lock first

Today `cpu::without_irqs` on one core is the exclusive critical section.
With two schedule CPUs that is **false**.

| Choice | First code era | Later residual |
| --- | --- | --- |
| Lock | **One big sched spinlock** (ticket or test-and-set) guarding `SCHED` / the pure table mutations that cross slots | Lock refinement / finer partition if measured need |
| Local IRQs | Always take the lock with **this CPU’s IRQs masked** (no sleep under lock; no voluntary switch while holding it) | Unchanged |
| Lock-free remote enqueue | Not first | Optional residual |

**Partition-only without a lock is rejected for the first era:** `states`,
epochs, admit, exit, caps, and wake still mutate shared rows even when
ready lists are per-core.

#### IPI ↔ lock (non-negotiable)

An SGI handler **must not** take the sched lock and **must not** call
`switch`.

| Who | May |
| --- | --- |
| SGI / device IRQ handler | Set a per-CPU `need_resched` (or equivalent kick) only |
| Voluntary path / IRQ **return epilogue** | Take lock, run `switch(cpu, …)`, release lock |

Same doctrine as ADR-0008 / ADR-0051: handlers do not context-switch; the
safe point does. If the primary holds the lock and sends a resched IPI,
core 1’s handler only pokes a flag; core 1 switches after EOI / when it
next leaves WFI on the idle path — never re-entering the lock from the
handler while the primary still holds it.

### 5. SGI 0 is RESCHED

[ADR-0074](0074-k8-ipi-wake-second-slice.md) delivered SGI 0 core0→core1 and
banked GICC bring-up. This design **promotes** that line from a one-shot
probe to the permanent **resched IPI**:

- Encoding and enable path stay as in 0074 (`kernel_core::gic::sgir_word`,
  banked enable on the target).
- Oracle line `smp: core1 ipi` remains valid evidence of delivery; later
  code may also print schedule-specific lines (`smp: core1 ran`, …).
- A second SGI id is **not** introduced here (TLB shootdown / other IPIs
  need their own ADR).

### 6. Dual idle

| CPU | Idle body | Stack |
| --- | --- | --- |
| 0 | Existing console idle (`console_loop`: RX, ticks, WFI) | Bootstrap / current idle slot 0 |
| 1 | **No console TX.** `poll_wakes` (if shared wake drain is safe under the lock rules) + WFI; never `with_tx` without a dedicated audit | Real slot (thin stack or existing core1 BSS stacks — code ADR chooses) |

Core 1 must not become a second serial console. ADR-0070’s “no console TX
from core 1” stays in force.

Secondary bring-up after MMU/VBAR (today `harbor_secondary_idle` in the
board IRQ bind) moves **schedule policy** into `sched` (or bootstrap): BSP
keeps banked GIC init only; arch still does not import board/drivers.

### 7. Preemption — still core 0 only until a later ADR

| CPU | First queues **code** slice | Later |
| --- | --- | --- |
| 0 | K4 EL0 + EL1 preemption unchanged | — |
| 1 | **Voluntary only** (yield / block / exit); no timer PPI, no quantum epilogue | Per-core timer + `SLICE_START[cpu]` + lift of the affinity fence for preemption |

Rationale: banked PPI timer + per-CPU quantum state is a second design
surface. Coupling it to the first queue landing multiplies failure modes.

The ADR-0074 `el1_preempt_pending` affinity fence **remains** until a
dedicated per-core preemption slice. Multi-current alone does not enable
K4 on core 1.

### 8. Spawn and wake (policy)

```text
spawn_on(cpu, entry, caps…)
  → admit(home=cpu) → enqueue Ready on queue[cpu]
  → if cpu != self: send resched IPI to cpu

wake(id)   // IPC, irq-wait, park timeout, …
  → if already Ready/Running: model decides (unchanged refusals)
  → else Ready on queue[home]
  → if home != self: resched IPI to home
```

Every existing wake producer (send path, `irq::wait`, park timeout,
cancel/reap) must eventually route through this rule. The first code ADR
lists the call sites; missing one is a silent stuck Blocked task on the
wrong core’s absence of a kick.

### 9. Evidence (for the first code ADR, not this design)

| Claim | Gate |
| --- | --- |
| Core 1 ran a pinned task | Boot oracle line (e.g. `smp: core1 ran`) |
| IPI path still live | `smp: core1 ipi` (0074) unless superseded by a stronger line |
| No product regression | Existing `boot-check` green on core 0 paths |
| Pure model | Host unit tests (+ bounded model if state space allows) |
| HW | Stamp residual until a Pi transcript pays it |

Work-steal oracles are out of scope.

## First implementation slice (follow-on code ADR)

Ordered sketch — paid only when the code ADR names evidence:

1. Pure multi-CPU `Tasks` (or equivalent) + host tests for the invariants in §2.
2. Sched spinlock + wire `current[2]` / `queue[2]` through `sched` (replace
   “IRQ mask alone = exclusive”).
3. Core 1 idle task body; enter it from secondary bring-up after GIC banked init.
4. `spawn_on(1, …)` lab/oracle worker that yields; primary may spawn it.
5. Remote wake → home queue + SGI resched flag; handler sets flag only.
6. Oracle `smp: core1 ran` (name exact in the code ADR).

Not in that slice: steal, cores 2–3, core 1 timer preemption, lock-free
queues, TLB IPI.

## Non-goals

- Work stealing / load balancing  
- Unparking affinity 2–3  
- Per-core timer / K4 preemption on core 1  
- Soft affinity  
- Lock-free or MCS-refined runqueues as a requirement of the first landing  
- Cross-core console / status TFT updates from core 1  
- Changing ADR-0006’s cooperative *class* (priorities, fair share beyond
  existing quantum on core 0)  
- Claiming product multi-core schedule before code evidence  

## Relationship to other ADRs

| ADR | Relationship |
| --- | --- |
| [0006](0006-cooperative-execution-model.md) | Unchanged cooperative class; multi-core is *where* a task runs, not a new scheduling class |
| [0008](0008-irq-handler-policy.md) | Reinforced: SGI handler does not switch |
| [0048](0048-k8-smp-design.md) | This document owns the queues residual; steal still later |
| [0051](0051-k4-irq-preemption-design.md) / [0064](0064-k4-el0-preemption-first-slice.md) / [0068](0068-k4-el1-preemption-second-slice.md) | Core 0 only until a per-core preemption ADR |
| [0070](0070-k8-smp-first-slice.md) | Unpark/alive prerequisite |
| [0074](0074-k8-ipi-wake-second-slice.md) | SGI path prerequisite; §4 fence stays for preemption; SGI 0 becomes RESCHED |

## Alternatives considered

| Alternative | Why not first |
| --- | --- |
| Single global queue + one lock | Simpler code, worse cache behaviour; fights ADR-0048’s stated per-core intent; steal later becomes a rewrite |
| Fully partitioned lock-free queues, no big lock | Correctness cost too high for first dual-core proof; states/caps still shared |
| Steal-first SMP | Product does not need it to prove two currents; explodes model and wake rules |
| Fold design into the first code ADR | Boundary choices would be review-only after the fact |

## Consequences

### Positive

- Dual-core schedule has a written mutual-exclusion and affinity story before
  code  
- Reuses paid IPI and unpark work instead of a third wake mechanism  
- Host-testable decision core stays the project’s default shape  

### Residual after first code

- Work stealing  
- Per-core preemption / timer on core 1  
- Lock refinement  
- HW stamp for schedule-on-core1  
- Full audit of every wake producer for remote kick  

### Gates (design-level)

| Reversal | Catch |
| --- | --- |
| Code lands without pure multi-current model | Review + missing host tests in code ADR |
| SGI handler takes sched lock | Deadlock under load; review against §4 |
| Core 1 prints on UART without TX policy | Console corruption; ADR-0070 rule |
| Status flipped to done without oracle | `roadmap-evidence` / boot-check |

## Related

- [0048](0048-k8-smp-design.md), [0070](0070-k8-smp-first-slice.md),
  [0074](0074-k8-ipi-wake-second-slice.md)  
- [0006](0006-cooperative-execution-model.md), [0008](0008-irq-handler-policy.md)  
- [0051](0051-k4-irq-preemption-design.md) (handler vs epilogue precedent)  
