---
id: 0006
title: Cooperative execution model (M3 tasks)
status: accepted
date: 2026-08-04
---

# ADR-0006: Cooperative execution model (M3 tasks)

## Acceptance status

**Accepted** as the execution model for M3 (human accept of the model before a
full scheduler). The decision is immutable from here: preemption, IRQ-side
context switches, `link.ld` task stacks, and software canaries instead of
unmapped guards are out of scope unless a successor ADR supersedes this one.

Accepting the **model** is not the same as marking M3 `done (HW)`. Pure
runqueue arithmetic may land under this ADR; the automated gates named under
“The gate that protects this decision” (interleaved yield on hardware, stack
overflow probe) still arrive with the rest of the milestone.

## Context

The kernel has a free-list heap, per-region permissions, guarded bootstrap and
exception stacks, and an event-driven `console_loop` that idles with `WFI`. It
has **no** unit of execution: one control flow on core 0 at EL1, one identity
map, no scheduler.

[Milestone M3](../architecture.md) is cooperative tasks. Its done-when is
concrete — two tasks yield on hardware with interleaved console output; each
stack is layout-validated; an overflow probe faults rather than entering another
task's stack. What it did not have was a recorded **model**. Finding F12 from
the [2026-08-04 multi-role review](../reviews/2026-08-04-multi-role.md) blocked
M3 for that reason alone: the dependencies existed, the abstraction did not.

[ADR-0001](0001-multi-role-analysis.md) requires that a finding which moves a
boundary become an ADR **before** the code that implements it. Writing tasks
first would make the execution model an artefact of the first implementation
that compiled — stack placement, preemption, and IRQ wake policy frozen by
accident.

This ADR is that boundary. It does not implement a scheduler.

## Decision

M3 introduces **tasks**: schedulable EL1 control flows. They are not agents
(agents remain M5+ vocabulary: address space, capabilities, user mode).

### Scheduling class: cooperative only

- A task runs until it calls **`yield`** or **exits**.
- There is **no preemption** on the arch timer. `time` stays a clocksource and
  tick counter; CNTP is not a scheduling quantum.
- **IRQ handlers must not context-switch.** They may only touch the same class
  of shared state they use today (atomics, rings). Any future wake (M4 / F13)
  posts work for the voluntary path; the switch happens on the next `yield` or
  idle reschedule.

Cooperative scheduling isolates the execution abstraction from timer-phase
correctness (F18) and from full trap-frame discipline on every IRQ.

### Idle

Exactly one **idle task**. Its body is what `bootstrap::console_loop` does
today: drain the UART RX ring, report ticks, and `WFI` under `without_irqs`
when there is no ready work. Post-bring-up bootstrap spawns any demo or work
tasks and then enters idle — there is no third ad-hoc loop outside the model.

### Ready queue

A fixed-capacity **FIFO / round-robin** runqueue. No priorities in M3.

- Pure queue arithmetic lives in **`kernel-core`** and is host-tested.
- The kernel crate owns TCBs, stacks, and the switch.

`yield`: if the current task remains runnable, enqueue it; dequeue the next;
switch. If the ready queue is empty, run idle.

### Context switch

Voluntary only. Save and restore the minimum needed for an EL1 cooperative
switch (callee-saved GPRs, SP, continuation) — **not** a full
[`TrapFrame`](../../src/arch/aarch64/exception/frame.rs) on every yield.
`TrapFrame` remains the exception path, coupled to `vectors.s`. The switch
critical section runs with IRQs masked, in the same spirit as the heap and
`irq::seal`.

### Stacks and guards

Architecture requires a per-task stack **from the heap**, with a **guard**, and
an overflow that **faults** rather than entering another stack.

- Each task stack is **heap-allocated**, page-aligned: `N` usable pages plus
  **one guard page** at the low address (stacks grow down).
- The guard is a **translation fault**, not a software canary. After
  allocation, **unmap** the guard page in the live kernel map. The physical
  page stays owned by the stack allocation (not returned to the free list), so
  the free-list is never handed a virtual hole.
- Spawn validates the stack+guard in the spirit of `mm::layout` /
  `GuardedStack` (extend the validator or keep a task-stack registry checked at
  spawn) so the M3 done-when is falsifiable.
- The bootstrap stack and exception stack in `link.ld` **remain** for early
  boot and `SP_EL1`. They are not task stacks.

Implementing the guard therefore requires an **unmap** (valid→invalid) path in
`arch::mmu` when M3 is coded. That path is also the first mapping change that
makes TLB maintenance observable (see [`verification.md`](../verification.md));
it is a consequence of this decision, not a reason to defer the model. If unmap
is judged too large mid-implementation, a **successor ADR** must change the
guard strategy — a silent canary is not an allowed retreat.

### Lifecycle (M3 minimum)

| State   | Meaning                                      |
| ------- | -------------------------------------------- |
| Ready   | On the runqueue                              |
| Running | The sole running task on core 0              |
| Exited  | Terminal; stack may be reclaimed             |

There is **no Blocked** state and no timed sleep in M3. Blocking on IPC or
events is M4 (and needs the IRQ/handler policy of F13). Sleeping for N ticks
needs a trustworthy deadline (F18). Keeping Blocked out of this ADR avoids
smuggling a half-mailbox into the execution model.

### Layering

```
bootstrap / console_loop   policy, idle body
        │
   sched (kernel)          TCB, spawn, yield, switch glue
        │
   kernel-core             runqueue arithmetic (+ host tests)
        │
   mm / arch::mmu          stack alloc + guard unmap
   arch::exception         voluntary switch does not go here
```

Drivers and `exception` still do not know tasks. Console TX ownership is
unchanged: multi-task printers serialize through the existing acquire path
until a later console-abstraction change.

### What this ADR does not decide

| Concern                         | Where it lives                          |
| ------------------------------- | --------------------------------------- |
| IRQ handler cookies / cap_irq   | F13, M4                                 |
| Absolute timer deadlines        | F18 (blocks time-based scheduling only) |
| Frame allocator / multi-AS / EL0| ADR-0005, M5                            |
| IPC and capabilities            | M4                                      |
| Preemption or SMP               | A future ADR that supersedes this one   |

## Consequences

**Positive** — M3 has a model that matches its done-when without inventing
preemption or IPC; pure runqueue logic is host-testable; stack overflow stays
in the same verification culture as the bootstrap guard (ESR on hardware);
IRQ and scheduling stay separated, so F13 can redesign handlers without undoing
the switch path; F18 is not a hidden dependency of every M3 line.

**Negative** — a runaway task that never yields starves the system (including
idle and the console); unmap is new surface area on the MMU path; fixed
runqueue capacity is a build-time limit; without Blocked, nothing waits on
events until M4.

## Alternatives considered

| Alternative                                      | Why not                                                                                                                                 |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
| Preemptive round-robin on CNTP                   | Pulls F18, full IRQ-side switch, and priority later; not required by the M3 done-when                                                   |
| Async/await or stacked futures as the only model | Hides stacks; cannot meet “overflow faults on a guard page” without reintroducing real stacks                                           |
| Keep a single `console_loop` until M5            | Contradicts the M3 milestone; no incremental path to agents                                                                             |
| Task stacks only in `link.ld`                    | Explicitly rejected by the M3 needs table; fixed count; no reclaim story                                                                |
| Software canary instead of an unmapped guard     | Fails the M3 overflow observable (translation fault, not a lucky panic)                                                               |
| Design full IPC / capabilities here              | That is M4; would freeze an ABI under the F13 vacuum                                                                                    |
| Context-switch from an IRQ handler on wake       | Breaks the exception layering rule; races with multi-claim IRQ entry                                                                    |
| Frame allocator for task stacks now              | ADR-0005: wrong shape until address spaces come and go (M5). Heap alloc + unmap is enough for guards in one AS                          |

## The gate that protects this decision

There is no disassembly gate for “still cooperative.”

| Layer        | What it catches                                                                 |
| ------------ | ------------------------------------------------------------------------------- |
| Process      | Incremental multi-role review (ADR-0001) before M3 is marked `done (HW)`        |
| Docs         | The M3 “Needs first” row names this ADR; supersession requires a successor ADR  |
| When M3 lands| Host tests on the runqueue; HW/QEMU interleaved yield; overflow probe ESR table |

This is a **declared weakness** relative to ADR-0002/0003: reversal is a review
and documentation failure mode, not an automatic red build, until the M3 tests
exist. Inventing a green check that does not constrain the model would be worse
than an honest gap.

## When to revisit

- **M4:** Blocked state, mailbox wake, F13 handler shape.
- **Any preemption or secondary core:** write a successor ADR; do not stretch
  this one.
- **M5:** EL0, per-task `TTBR0`/ASID, lazy FP (see ADR-0002).
- **If M3 needs sleep-on-ticks before F18 is fixed:** fix F18 first, or
  risk-accept drift in an explicit amendment — silent drift is not acceptable.

## References

- [`architecture.md`](../architecture.md) — M3 needs / done-when; F12
- [`reviews/2026-08-04-multi-role.md`](../reviews/2026-08-04-multi-role.md) — R10 / F12
- [ADR-0001](0001-multi-role-analysis.md) — boundary findings become ADRs first
- [ADR-0005](0005-static-page-table-arena.md) — why not a frame allocator here
- [`verification.md`](../verification.md) — guard probes; TLB maintenance gap
- `src/bootstrap/console_loop.rs`, `src/mm/mod.rs`, `src/arch/aarch64/exception/frame.rs`
