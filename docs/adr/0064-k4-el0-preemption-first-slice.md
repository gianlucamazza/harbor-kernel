---
id: 0064
title: K4 first code slice — lower-EL (EL0) IRQ preemption
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0006, 0017, 0022, 0028, 0046, 0051, 0057, 0062]
---

# ADR-0064: EL0 IRQ preemption (K4 first code slice)

## Acceptance status

**Accepted** (2026-08-09), on delegated authority: the owner delegated
acceptance of the ADR-0051 follow-on implementation ADR when choosing "K4
preemption first code" as the next roadmap step (session 2026-08-09).

Implements the first **code** slice of the
[ADR-0051](0051-k4-irq-preemption-design.md) design. Scope is **lower-EL
only**: an EL0 session that never traps loses the CPU when its quantum
expires. Same-EL (EL1) preemption stays deferred — see §5 residuals.

## Context

ADR-0051 accepted the preemption _design_: the timer tick sets a
`need_resched` flag when the current task's quantum (same arithmetic as the
[ADR-0046](0046-k4-cooperative-cpu-budget.md) budget) is expired; the switch
runs on the IRQ **return epilogue** after GIC EOI, never deep in the device
IRQ path; `cpu::without_irqs` regions stay non-preemptible; idle is never
preempted into idle. It deferred code until the trap-frame → TCB discipline
was named, and mandated a re-audit of the cooperative-atomicity assumptions
it invalidates.

## Decision: lower-EL first

Preempting an **EL1** task means its full state lives in the `TrapFrame` on
the shared `SP_EL1` exception stack; switching away with that frame live
requires a per-TCB frame save area, an epilogue stack pivot, and an `eret`
resume trampoline coexisting with the callee-saved `Context` path. That is
real new assembly and a new task-resume mode.

For **EL0** the trap-frame → TCB problem is already solved:
`exception_irq_el0` copies the full frame into the live `El0Session`
(`saved.gpr/elr/spsr`, `saved_sp_el0`) and the vector unwinds the exception
stack before `el0_run_finish`. Control then returns to the agent host loop —
an ordinary EL1 task on its own stack — whose `El0Outcome::Irq` arm runs
`irq::handle_cpu_irq()` (claim → dispatch → EOI) and resumes. Checking the
flag between EOI and resume **is** the lower-EL IRQ-return epilogue of the
ADR-0051 design, reachable with the existing voluntary switch machinery: no
new assembly, no `vectors.s` change.

This slice also closes a documented evidence gap: the cooperative budget
does not cover a spinning EL0 agent (`verification.md`, SECURITY.md attacker
model). An EL0 spinner that makes no syscalls and still loses the CPU is
exactly the missing observable. Preempting the EL1 `budget_worker_a/b` would
prove less — they already rotate voluntarily.

### Mechanism

| Piece             | Shape                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Pure model        | `kernel_core::preempt::should_set(slice_start, now, quantum, current_is_idle)` delegating to `budget::expired`; host-tested                                                                                                                                                                                                                                                                                                              |
| Model switch kind | `kernel_core::tasks::Switch::Preempt` — rotation identical to `Yield`; `Stay` when the runqueue is empty or current is idle (never enqueue idle)                                                                                                                                                                                                                                                                                         |
| No flag carrier   | ADR-0051 sketched a `need_resched` flag set by the tick handler. The predicate is **monotone in `now`** — once the expiry tick fires it stays true until the switch resets `SLICE_START` — so the safe point evaluates it directly. Deliberate deviation: it deletes flag staleness (a flag outliving the slice that earned it) and keeps the `time → sched` layering edge closed (the `layering` gate refuses `time` importing `sched`) |
| Safe point        | `sched::preempt_switch()`: evaluate `should_set`; if true, `switch_with(Switch::Preempt)`. Called from the agent loop's `resume_step_preemptible`, **before** the `without_irqs` mask is taken (ADR-0022 one-step rule; the `irq-scope` gate sees it outside the region)                                                                                                                                                                 |
| Idle              | `CURRENT_IS_IDLE` mirrored where `SLICE_START` is written on switch-in, so the safe point reads both without opening `SCHED`; idle never rotates                                                                                                                                                                                                                                                                                         |
| Quantum           | `BUDGET_QUANTUM_TICKS` (2 ticks) and `SLICE_START` — unchanged from ADR-0046; preemption is the IRQ-epilogue enforcement of the same quantum                                                                                                                                                                                                                                                                                             |

## What stays true

- **ADR-0006 is not amended by this slice.** No context switch happens in
  exception context — the IRQ handlers are untouched; the switch runs in
  task context at a safe point. The partial supersession of ADR-0006 arrives
  with the same-EL slice.
- **ADR-0022 one-step mask** is untouched: the preempt check sits outside
  the masked resume step.
- `SLICE_START.store` on switch-in already restarts the quantum for the
  next task; a preempt switch is a switch like any other.

## Re-audit (mandated by ADR-0051)

1. **`sched::transfer_held_to_peer` four masked regions.** In this slice the
   predicate is evaluated only at the agent-loop resume boundary. An IRQ
   landing in one of transfer's unmasked gaps returns without switching; no
   switch runs before the syscall arm completes and reaches
   `resume_step_preemptible`. Lookup→move cannot be split. **Re-opened by
   the same-EL slice.**
2. **`switch(Exit)` → `taskcap::revoke_task` window (ADR-0057).**
   `switch_with` holds the mask from `irq_save` through the revoke region to
   `irq_restore`; nothing in this slice switches from IRQ context, and the
   safe point cannot run masked — the window is unreachable. The
   [ADR-0062](0062-taskid-epoch.md) epoch is defense in depth.
3. **Staleness.** There is no flag to go stale: the predicate reads the live
   `SLICE_START` of whoever is current, so a task that just switched in is
   never charged for its predecessor's overrun.
4. **Publication ordering.** `SLICE_START` and `CURRENT_IS_IDLE` are written
   together under the switch mask and read by the safe point only in task
   context on the same core — no torn pair is observable. Relaxed atomics
   suffice on this single-core slice; K8 revisits.

## Evidence

| Check                                                     | Gate                                                |
| --------------------------------------------------------- | --------------------------------------------------- |
| Host model of the preempt predicate + `Switch::Preempt`   | kernel-core unit tests                              |
| Non-syscalling EL0 spinner loses the CPU; peer progresses | QEMU `preempt: rotated` (boot-oracle, both runners) |
| Budget oracle still green in the same boot                | `budget: rotated`                                   |
| No switch under `without_irqs`                            | `irq-scope` (`preempt_switch` added to SWITCHERS)   |

The demo terminates deterministically: the peer proves rotation
(`PEER_TURNS >= 2` while the spinner is live), prints the oracle, then
writes a stop word into the spinner's user page; the spinner observes it and
exits via `SYS_EXIT`. No iteration-count timing guess.

## Residuals

- **Same-EL (EL1) preemption** — the ADR-0051 trap-frame → TCB slice:
  per-TCB frame save area modeled on `El0Session.saved`, epilogue pivot off
  `SP_EL1`, `eret` resume trampoline. Requires re-running re-audit items 1–2.
- **HW stamp** — `preempt: rotated` on Pi silicon via the serial transcript
  loop; the roadmap row stays `done (QEMU)` until then.
- Per-agent budgets, priority (unchanged from ADR-0046 residuals).
