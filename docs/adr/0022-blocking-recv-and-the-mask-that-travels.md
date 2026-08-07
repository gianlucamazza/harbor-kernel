---
id: 0022
title: Blocking SYS_RECV — an agent parks, and the interrupt mask stops travelling with the switch
status: proposed
date: 2026-08-07
related: [0006, 0008, 0017, 0018, 0019]
---

# ADR-0022: A parked agent, and the `without_irqs` that cannot span a switch

## Acceptance status

**Proposed** (2026-08-07). Deferred deliberately by
[ADR-0017](0017-el0-capability-abi.md), whose consequences say a blocking recv
is the change that would make an agent yield out of a live EL0 session, and that
M7 does not do it.

## Context

`SYS_RECV` returns `Status::Empty` and resumes. `ipc::try_recv_from_slot` says
why in its own doc-comment:

> Non-blocking by construction: a blocking recv would have to yield out of a
> live EL0 session, and nothing performs a switch inside one yet.

The consequence is visible in the oracle. `demos.rs` orders the two EL0 agents
by hand — _"let the sender post first; `SYS_RECV` never parks, so this is
ordering by construction"_ — and a receiver that arrives early has to spin. An
agent cannot wait for work; it can only be scheduled after the work exists.
That is a demo constraint pretending to be a design.

### What already exists, measured rather than assumed

Most of a blocking recv is built, for the EL1 side and by ADR-0019's clean-up:

| Piece                                           | Where                                   | State                                                                   |
| ----------------------------------------------- | --------------------------------------- | ----------------------------------------------------------------------- |
| A task state for waiting                        | `tasks::State::Blocked`                 | Exists, exercised by unit tests and by the bounded model                |
| Park bookkeeping that reports instead of acting | `ipc::Table::park` / `Table::send`      | `send` returns `Ok(Some(waiter))`; the table never calls the scheduler  |
| The wake                                        | `sched::wake_task`                      | Used by `ipc::send`, and reached from EL0 through `ipc::send_from_slot` |
| A blocking recv for kernel tasks                | `ipc::recv`                             | Loops `try_recv` → `park` → `sched::block_current`                      |
| A session that survives a switch                | `sched::publish_el0` on the switch path | Stamped on silicon 2026-08-07: the atomic holds on the vector path      |

So the missing piece is not the scheduler and not the session. It is one
structural fact about the loop that drives EL0.

### The fact: the mask is a property of the core, not of the task

`Agent::run_user_prog_resuming_prep` wraps the **entire session** — enter, every
SVC, every resume, until exit or fault — in a single `cpu::without_irqs`. And
`without_irqs` is:

```rust
pub fn without_irqs<R>(f: impl FnOnce() -> R) -> R {
    let daif = irq_save();
    let result = f();
    unsafe { irq_restore(daif) };
    result
}
```

`irq_save` reads `DAIF` **now** and `irq_restore` writes it back **later**. If a
switch happens between the two, the value crosses into another task's execution:
the next task runs with this task's mask, and when this task eventually resumes
it restores a `DAIF` captured in an epoch that has ended. Calling
`sched::block_current()` from inside that closure is exactly that switch.

This is not a subtle race to be closed with an ordering argument. It is a
scoping error that the current code avoids only because nothing inside the scope
ever switches — which is precisely what this ADR proposes to change.

## Decision

**1. `SYS_RECV` on an empty mailbox parks the calling agent's task.**

The agent's own text is unchanged and unaware: it executes `svc SYS_RECV` and,
whenever it next runs, finds `x0 = Ok` with the payload in `x1..x3`. Waiting is
the kernel's, not the program's. `Status::Empty` does not disappear — see §4.

**2. The masked region shrinks from the session to the step.**

`without_irqs` stops spanning the loop and wraps each `enter`/`resume` and the
register access around it. The loop body between two steps runs unmasked, and a
park happens there. The rule this ADR states, and which the code must be shaped
to make structurally true rather than remembered:

> A `DAIF` save/restore pair must not span a call that can switch tasks.

**3. The park reuses `ipc::recv`'s sequence exactly, including the re-check.**

`try_recv`, then `Table::park` under the mask, then `block_current` only if
`park` returned `None`. The re-check inside `park` is not defensive coding: the
mask is dropped between `try_recv` and `park`, so a message can land in that
window, and `park` returning `Ok(Some(msg))` is that case. Duplicating the
sequence for EL0 would be duplicating a subtlety; the EL0 path calls the same
table operations in the same order.

**4. `Busy` stays a refusal, and `Empty` stays reachable.**

A mailbox already has a waiter (`Table::park` → `RecvError::Busy`) is a state
refusal, counted as one, and returned to the agent — one endpoint, one waiter is
[ADR-0017](0017-el0-capability-abi.md)'s topology and this ADR does not widen
it. `Status::Empty` remains the answer to a **non-blocking** recv, which stays
in the ABI as a separate immediate: an agent that must not park (an interrupt
service loop, a poll) needs to say so, and a blocking-only recv would take that
away.

**5. The idle task never parks, and this is checked rather than documented.**

`ipc::recv` already carries _"must not be called from idle or an IRQ handler"_ as
prose. Idle blocking is the one way to reach a core with nothing runnable. The
EL0 path gets the same rule as a refusal — idle asking to park is a state
refusal, not a panic and not a comment.

**6. The kernel does not resume a session it has not re-published.**

Nothing new is required: `require_published` already panics if the loop acts on
a session that is not the running task's, and the scheduler already publishes on
every switch. What changes is that this tripwire moves from _never fires_ to
_load-bearing_ — the park/wake round trip is the first code path that leaves and
re-enters a live session. It has now been exercised on silicon for switches, but
never for a switch **inside** a session.

## Consequences

### Positive

- An agent can wait for work instead of being scheduled after it. That is the
  difference between a demo ordering and a system.
- The oracle stops hand-ordering the two EL0 agents, so the boot check starts
  asserting the property (_the receiver arrives first and still gets the
  message_) instead of arranging for it not to be tested.
- The `DAIF` scoping rule becomes explicit, and it is the kind of defect that is
  invisible in review and silent at runtime until an interrupt is lost.
- `sched::publish_el0` gets the exercise ADR-0019's hardware stamp could not
  give it: a switch with a live session on both sides.

### Negative / debt

- **A parked agent holds its address space and its frames.** Nothing reclaims
  them and nothing should — but an agent that parks on a channel no one holds
  the send end of is a leak with no diagnostic. There is no timeout and this ADR
  does not add one.
- **No priority, still.** The woken task goes to the back of the ready queue.
  [ADR-0006](0006-cooperative-execution-model.md)'s cooperative model is
  untouched: a park is a voluntary yield with a reason.
- **The unmasked window is new surface.** Between two steps of the loop the task
  can now be preempted by a timer IRQ while an EL0 session is live but not
  running. `El0Session` is per-task and the pointer is republished on switch, so
  this is sound by ADR-0017 §1 — but it is sound by argument, and the argument
  is now doing work it was not doing before.
- **`SessionStats` spans a park.** The counters an agent's session accumulates
  now cover wall-clock in which other tasks ran. Anything reading them as a
  duration is wrong, and nothing currently does.

### Gates that catch reversal

| Reversal                                            | Gate                                                                                                                                                                     |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| A `without_irqs` grows back around a switching call | A new check in the `make check` family, greppable in shape: `block_current` / `wake_task` / `switch` must not appear lexically inside a `without_irqs` closure in `src/` |
| The park is skipped and `Empty` returned again      | `boot-check`: the oracle's receiver is spawned **before** the sender and must still print `got payload`. Ordering by construction stops being available                  |
| Two waiters on one endpoint                         | `Table::park` returns `Busy`; the bounded model in `model_ipc.rs` already walks park/send/recv against a reference implementation and would report the divergence        |
| The session is resumed on the wrong task            | `require_published` panics; `boot-check` and the hardware stamp assert zero panics                                                                                       |
| Idle parks                                          | Host test: `park` from `Tasks::IDLE` is a state refusal, and the scheduler model's invariant _idle is never off the run queue_ already holds it                          |

### Seen red before green

Each of the five above must be observed failing before it is trusted, per
[`verification.md`](../verification.md). The one that matters most is the first:
a gate on lexical scope is easy to write in a form that never matches anything.

## Alternatives rejected

| Alternative                                                       | Why not                                                                                                                                                                                               |
| ----------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Keep `Empty` and let agents spin                                  | It is the status quo, and it is why the oracle orders its agents by hand. A spinning agent also burns the timeslice it is waiting for                                                                 |
| Park inside the existing `without_irqs`, restoring `DAIF` by hand | Trades a scoping rule for a discipline. The save/restore pair exists so nobody has to remember; reaching under it to switch is how the pair stops meaning anything                                    |
| A separate `SYS_WAIT` beside the non-blocking `SYS_RECV`          | Two syscalls where one immediate distinguishes them. The ABI already carries an immediate per operation, and the reply shape is identical — §4 keeps both behaviours without a second entry point     |
| Wake with a timeout                                               | Needs a deadline queue and a second reason a task can leave `Blocked`. Worth its own decision; folding it in here would make the park's contract two things at once                                   |
| Preemptive scheduling first, so the park is unnecessary           | Preemption is an explicit non-goal until its own ADR ([`architecture.md`](../architecture.md)). A parked task is not a scheduling policy — it is a task with nothing to do, which is a different fact |
