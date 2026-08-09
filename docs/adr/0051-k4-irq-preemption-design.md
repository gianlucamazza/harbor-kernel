---
id: 0051
title: K4 design — IRQ-side preemption (implemented in ADR-0064 / ADR-0068)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
amended: 2026-08-09
related: [0006, 0046, 0026, 0064, 0068]
---

# ADR-0051: IRQ preemption design (K4 — design; code in ADR-0064 / ADR-0068)

## Acceptance status

**Accepted as design** (2026-08-08). **Code complete** in
[ADR-0064](0064-k4-el0-preemption-first-slice.md) (EL0) and
[ADR-0068](0068-k4-el1-preemption-second-slice.md) (EL1), both **done (HW)**.
This ADR remains the design record; [ADR-0006](0006-cooperative-execution-model.md)
is partially superseded for the IRQ-epilogue path (device handlers still never
switch). Cooperative budget ([ADR-0046](0046-k4-cooperative-cpu-budget.md))
remains the voluntary fairness path under the same quantum arithmetic.
Present-tense status is **implemented**, not “code deferred”.

## Context

At design time a task that never yields could starve peers until it hit a
voluntary checkpoint (`yield_if_budget_expired`, park, exit). Budget closed the
lab case; production boundary OS fairness wanted the **timer IRQ** to force a
reschedule without cooperation.

ADR-0006 forbade IRQ handlers from switching. This document is the successor
_design_; implementation landed in dedicated ADRs after trap-frame discipline
was named (ADR-0064 lower-EL, ADR-0068 same-EL).

## Decision (design)

| Item                     | Intent                                                                                                                                                          |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Trigger                  | Arch timer tick when current task's quantum is expired                                                                                                          |
| Where switch runs        | **Not** deep in the device IRQ path: set a `need_resched` flag in the timer handler; switch on the **IRQ return** epilogue (same-EL and lower-EL) after GIC EOI |
| Saved state              | Full GPR + ELR/SPSR already in `TrapFrame` / `El0Session`; reuse EL0 session layout for lower-EL preemption                                                     |
| Kernel critical sections | `cpu::without_irqs` regions remain non-preemptible; no switch while a mask is held by the voluntary path                                                        |
| Interaction with budget  | Quantum = same tick arithmetic as ADR-0046; preemption is the IRQ-side enforcement of the same quantum                                                          |
| Idle                     | Never preempt idle into idle; if only idle is ready, clear flag and return                                                                                      |
| Evidence                 | QEMU + HW: non-yielding spinner loses the CPU (`preempt: rotated` / `preempt-el1: rotated`); host model of quantum predicate                                    |

### First implementation slice (historical sketch; paid)

1. Pure `need_resched` / quantum arithmetic (if any beyond budget).
2. Timer handler sets flag when `budget_expired()`.
3. EL1h IRQ return path checks flag → `sched::preempt_switch` (save trap frame into TCB, pick next, restore).
4. Lower-EL IRQ: after `exception_irq_el0` classification, optional forced yield before resume.
5. Oracle: spinner without yield; peer progresses.

Paid by ADR-0064 (lower-EL) and ADR-0068 (same-EL), with the amendment deviation
below (no `need_resched` flag).

### Non-goals of this design document (still open elsewhere)

- Priority scheduling.
- SMP IPI preemption (K8).
- Softirq / deferred work beyond today's wake queue.

(Implementation of the switch is **done** — not a residual of this ADR.)

## Relationship to ADR-0006

| ADR-0006 rule          | This design                                      |
| ---------------------- | ------------------------------------------------ |
| No IRQ context switch  | Superseded on the IRQ-return epilogue (code live)|
| Voluntary yield / park | Remain primary; preemption is the backstop       |
| Idle WFI model         | Unchanged                                        |

## Gates (satisfied)

| Check                          | Evidence                                                    |
| ------------------------------ | ----------------------------------------------------------- |
| Non-yielding task loses CPU    | QEMU + HW named oracles (`preempt: rotated`, `preempt-el1: rotated`) |
| No switch under `without_irqs` | `irq-scope` + review                                        |
| Budget arithmetic still live   | Same quantum under both preemption paths                    |

## Implementation and re-audit

Historical deferral was: trap-frame → TCB path against `vectors.s` /
`el0_run_finish` without breaking EL0 session invariants (ADR-0016/0017).

**Re-audit when code lands** (amended 2026-08-08, **closed**): every
cooperative-atomicity assumption this design invalidates — the four separate
masked regions of `sched::transfer_held_to_peer` (lookup → move is not atomic
under preemption), and the `switch(Exit)` → `taskcap::revoke_task` window
([ADR-0057](0057-taskcap-lifecycle.md) spawn-epoch residual).

> **Amendment (2026-08-09, reconciliation per ADR-0058).** Fully
> implemented: the lower-EL slice by [ADR-0064](0064-k4-el0-preemption-first-slice.md),
> the same-EL slice by [ADR-0068](0068-k4-el1-preemption-second-slice.md),
> whose re-audit section closes the clause above (linearization argument
> for the transfer gaps; the exit→revoke window is DAIF-gated). One
> deviation from this design's sketch, decided in ADR-0064: no
> `need_resched` flag — the predicate is monotone, so the epilogue
> evaluates it directly. No open same-EL re-audit remains.
