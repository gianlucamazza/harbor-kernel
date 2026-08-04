# Multi-role analysis (incremental) — M3 cooperative path

**Date:** 2026-08-04  
**Tree:** `4b573ba` + follow-ups (sched, task stacks, unmap)  
**Scope:** R1, R2, R3, R4, R5, R10 only — not a full R1–R12 re-baseline.  
**Method:** read-only review of `sched/`, `mm/task_stack`, `arch/switch`, `mmu::unmap`, idle/`with_tx`.  
**Hardware:** closed **after** this desk pass (2026-08-04 boot + 2026-08-05
overflow probe — see `verification.md`). This file’s body below is the original
desk analysis; the checklist at the end is the current status.

Per [ADR-0001](../adr/0001-multi-role-analysis.md): findings that move a boundary become ADRs before code. M3 model already has ADR-0006; F13 → [ADR-0008](../adr/0008-irq-handler-policy.md) (proposed).

---

## Executive summary

The cooperative path matches ADR-0006 on QEMU: voluntary switch, heap stacks with unmapped guards, idle = console loop, no IRQ-side switch. Layering holds (`sched` → `arch`+`mm` only).

**Update:** M3-H1 closed on silicon (interleave + scheduled overflow ESR). No P0
hang-class bug found in the desk review. Residual notes below; several (M3-S1
DAIF, `MAX_TASKS`) were addressed in later commits (`irq_save`/`irq_restore`,
`MAX_TASKS = 8`).

---

## Findings

| Sev | ID | Role | Title | Action | Effort |
| --- | --- | --- | --- | --- | --- |
| P1 | M3-H1 | R5/R10 | No HW transcript / overflow ESR for task guards | **Closed** 2026-08-05 | — |
| P2 | M3-S1 | R3 | `yield_now` used hard `irq_disable`/`enable` | **Closed** — `irq_save`/`irq_restore` across switch | — |
| P2 | M3-S2 | R3/R4 | `with_tx` holds IRQs masked for full UART TX | Accept short lines; no TX in IRQ | — |
| P2 | M3-S3 | R2 | Task stack `Drop` leaks by design if not `release`d | Document; exit path calls `release` | — |
| P2 | M3-S4 | R10 | Demo tasks only; no stress (many spawn/exit cycles) | Optional soak later | S |
| P3 | M3-S5 | R1 | Architecture diagram was stale (“scheduler from M3”) | Fixed in same hygiene pass | S |
| — | F18 | R3 | Was open for TVAL drift | **Resolved** (CVAL) earlier; docs updated | — |
| — | F13 | R3/R10 | Handler `fn()` | ADR-0008 proposed; blocks M4 not M3 | M |

---

## Per role (delta)

### R1 — Layering

- `sched` allowed edges: `arch`, `mm` only — clean.
- `exception` still only `irq` — clean.
- `kprintln` is macro ubiquity in layering script — OK.
- **Risk-accepted:** non-import coupling (GIC id constants) unchanged.

### R2 — Memory / MMU

- Task stacks: page-aligned heap + `unmap` guard + `validate_guarded_stack` — matches ADR-0006.
- Block split on unmap is required for heap-under-L2; smoke on every production boot.
- **M3-H1:** guard fault not yet ESR-tabled on silicon for *task* stacks (bootstrap guard was).

### R3 — IRQ / idle / concurrency

- Idle yields when `has_ready()` — workers not starved by perpetual WFI.
- IRQ handlers do not call sched — holds.
- **M3-S1:** `irq_enable()` after switch assumes callers wanted IRQs on; correct for current bootstrap (IRQs enabled before idle). Do not call `yield_now` from a masked critical section expecting to stay masked.
- Timer absolute deadline (F18) does not interact with cooperative yield.

### R4 — unsafe / panic

- Switch asm is minimal callee-saved; first entry via trampoline.
- Panic still `steal`s console; shared `TX` cleared — good.
- Exit releases stack after marking Empty; switch never returns to exited task — OK if no second core.

### R5 — Verification

- Host: runqueue + layout + decode_leaf tests.
- QEMU: boot-check asserts task-a/b lines + unmap smoke + ticks.
- **Blind:** TLB necessity on unmap still best proved by guard write fault (bringup probe).
- **Blind:** silicon attributes / GIC / CNTFRQ — need T1/T2 lab.

### R10 — Roadmap

- M3 code exists; mark `done (HW)` only after M3-H1.
- M4 blocked on ADR-0008 accept + implementation of wake queue + Blocked.
- Do not stretch ADR-0006 into preemption.

---

## Accepted risks (M3)

| Risk | Why now | Revisit |
| --- | --- | --- |
| Runaway task never yields | ADR-0006 | Preemption ADR |
| `MAX_TASKS = 8` | Bringup margin (was 4) | When spawn demand grows |
| Per-task DAIF restore | After `irq_save` fix | SMP |
| Bringup probe panics | Intentional; not in production image | After ESR recorded |

---

## Checklist to mark M3 `done (HW)`

1. [x] Pi 4B serial: `task-a`/`task-b` interleaved, `CNTFRQ=54000000`, DTB mapped (2026-08-04)  
2. [x] bringup: scheduled overflow probe, ESR translation L3, FAR in **self** guard, peers live (2026-08-05)  
3. [x] Boot transcript in `docs/verification.md`  
4. [x] Guard ESR row in `docs/verification.md`  

**M3 = done (HW)** as of 2026-08-05.

---

*Incremental re-run: after sched/mmu/switch changes, or before claiming M3 done (HW).*
