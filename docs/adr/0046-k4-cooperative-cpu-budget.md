---
id: 0046
title: K4 first slice — cooperative CPU budget (no IRQ preemption)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
amended: 2026-08-09
related: [0006, 0023, 0026]
---

# ADR-0046: Cooperative CPU budget (K4 entry)

## Acceptance status

**Accepted** (2026-08-08). First slice of **K4**: a **tick quantum** bounds how
long a task may run between voluntary checkpoints without inventing IRQ-side
context switches ([ADR-0006](0006-cooperative-execution-model.md) remains in
force until a true preemption successor).

## Decision

### 1. Pure `kernel_core::budget`

`expired(slice_start, now, quantum_ticks) -> bool` — host-tested.

### 2. Kernel

- On switch-in, record `slice_start = time::ticks()`.
- `sched::budget_expired()` for workers, which hand-roll the yield loop (a
  `yield_if_budget_expired` helper never landed — amended 2026-08-08 to match
  the code, ADR-0058 convention).
- Default quantum: 2 ticks (TIMER_HZ=10 → ~200 ms class).
- **No** context switch from the timer IRQ.

### 3. Oracle

Two thin tasks spin checking `budget_expired`; interleaved progress prints
`budget: rotated`.

### 4. Residuals

True preemption (IRQ switch) — design in [ADR-0051](0051-k4-irq-preemption-design.md);
per-agent budgets, priority.

## Gates

| Check                      | Evidence          |
| -------------------------- | ----------------- |
| Host expired arithmetic    | unit tests        |
| QEMU rotation under budget | `budget: rotated` |

> **Amendment (2026-08-09, reconciliation per ADR-0058).** The
> `budget: rotated` oracle and its workers are **superseded by
> [ADR-0068](0068-k4-el1-preemption-second-slice.md)**: once the IRQ
> epilogue enforces the same quantum, the tick that made the workers'
> voluntary check true rotates them first by construction — the cooperative
> observation window can never open again. The replacement oracle
> `preempt-el1: rotated` proves a strictly stronger claim (rotation without
> cooperation). `sched::budget_expired` left with the workers; the quantum
> arithmetic lives on in `kernel_core::budget` under both preemption paths.
