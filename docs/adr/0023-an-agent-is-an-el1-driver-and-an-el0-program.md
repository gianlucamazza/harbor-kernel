---
id: 0023
title: An agent is two things — an EL1 driver task and an EL0 program — and this records the shape rather than changing it
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0006, 0016, 0017, 0018, 0021, 0022, 0024]
---

# ADR-0023: The schedulable entity is the driver, not the agent

## Acceptance status

**Accepted** (2026-08-07) by the project owner, as drafted, with two number
fixes at acceptance: `MAX_TASKS` is **16** after M8 (was 14 when proposed), and
M8's EL1 console server is **not** an agent pair — it is a plain driver-less
kernel task that parks on a mailbox (see [ADR-0024](0024-parked-task-visibility.md)).

This ADR decides almost nothing. It **states** a structure that six accepted
ADRs assume and none of them writes down, and it names what that structure costs
and precludes — so the next decision that would build on it (preemption, an
agent that outlives its creator, a per-agent identity, reaping a parked
driver) starts from a described shape instead of an inferred one.

## Context

Ask "what is an agent in Harbor" and the code gives two answers.

**The EL0 program** is what ADR-0017's authority model is about: a private
address space, a capability table, a slot ABI, text that cannot name anything
outside its own array.

**The EL1 task** is what the scheduler runs. `Agent::run_user_prog_resuming`
(`src/agent/mod.rs`) is a synchronous loop owned by a kernel task: it enters
EL0, handles each SVC, resumes, and returns when the session ends. The task
exists for the whole session and is what `sched` admits, switches to, and
accounts for.

So an agent is a **pair**, and the schedulable entity is the second half.

### What the pair costs, measured

| Per agent         | Cost                                                                  |
| ----------------- | --------------------------------------------------------------------- |
| One `sched` slot  | `MAX_TASKS = 16`, machine-wide, shared with every EL1 demo and the console server |
| One kernel stack  | `TASK_STACK_USABLE = 16 KiB` on the heap, plus an unmapped guard page |
| One `El0Session`  | In the TCB (ADR-0017 §1)                                              |
| One address space | Root, cloned kernel tables, and its window's frames                   |

The kernel stack is the one worth staring at. It exists so an EL1 loop can sit
in a call frame across the session — not because the EL0 program needs it. An
agent whose entire text is 32 bytes still costs 16 KiB of kernel stack.

### Three consequences that look unrelated and are the same thing

**Preemption is not one change.** [ADR-0006](0006-cooperative-execution-model.md)
names preemption a non-goal, and the usual reading is "the scheduler is
cooperative". The stronger fact is that a preemptive scheduler would have to
preempt the **driver**, mid-loop, between an SVC and its resume — with an EL0
session live and `CURRENT_EL0` published. That is a different problem from
preempting a plain kernel task, and it is invisible until you know the driver is
the schedulable thing.

**"The creator decides" is ambiguous by one level.**
[ADR-0018](0018-agent-fault-policy.md) says the kernel ends the session and the
creator decides what happens to the task. But the driver _is_ a task, and the
agent is not one — so "the task" means the driver, and an agent cannot be killed
without killing the loop that was watching it. `pl011-agent: killed ok` works
because the driver kills itself.

**`MAX_TASKS` is scarce for a reason nobody states.** It went 12 → 14 when the
loader landed, because two manifest entries needed two _driver_ tasks, then
14 → 16 for M8's always-on console server plus product beacon. A design where
the EL0 context were the schedulable entity would not have spent a task slot on
the loop that drives each agent.

### Why this was never written down

Each ADR was locally right. ADR-0006 decided cooperative scheduling before EL0
existed. ADR-0016 decided the session protocol with one machine-wide slot.
ADR-0017 moved the session into the TCB — which is the moment the pairing became
structural, because a session _belongs to a task_. ADR-0021 and ADR-0022 then
built on it. No single decision introduced the shape, so no single decision
recorded it.

## Decision

**1. The pair is the shape, and it stays for now.**

An agent is an EL1 driver task plus the EL0 program it drives. This ADR does not
propose collapsing them. The alternative — the EL0 context as the schedulable
entity, with the kernel returning to the scheduler on every exception — is a
different kernel, and adopting it would re-decide ADR-0006, ADR-0016, ADR-0017
and ADR-0018 at once.

**2. The word "agent" means the pair, and documents must not use it for
either half alone.**

Where the distinction matters — fault policy, lifetime, cost — the text says
_driver task_ or _EL0 program_. `docs/architecture.md`'s agent model gets the
pair as a named concept rather than a row that quietly means one of the two.

**3. A decision that depends on the pair must name it.**

Preemption, agent lifetime beyond its creator, per-agent identity, and reaping a
parked agent (issue #13) all turn on which half is being talked about. Each
should say so in its own Context rather than inheriting the ambiguity.

**4. The measurement is part of the record.**

16 KiB of kernel stack and one of sixteen task slots per agent is the price of
the current shape. Any future proposal to collapse the pair is arguing against
those numbers, and they belong here so the argument can be made against
something.

## Consequences

### Positive

- The next reader learns in one place what six ADRs assume.
- Preemption stops looking like a scheduler change. It is a change to what a
  session is.
- ADR-0018's "creator decides the task" acquires a referent.
- The cost of an agent is written where a proposal to reduce it can find it.

### Negative / debt

- **This ADR changes no code, which makes it the kind of document that rots.**
  Its only defence is that the code it describes is stable and heavily gated. If
  the driver loop moves or the session stops living in the TCB, this becomes
  wrong in a way no gate can see — it states a _shape_, and shapes are exactly
  what `make layering` and `make doc-symbols` cannot check.
- **It legitimises the cost by writing it down.** A number in an ADR reads as
  accepted. 16 KiB of kernel stack per agent is not a design goal, it is what a
  synchronous driver loop needs, and a system with many small agents would pay it
  badly.
- **It does not resolve the ambiguity it names, it only marks it.** Every place
  the word "agent" is load-bearing still has to be read carefully.

### Gates that catch reversal

| Reversal                                               | Gate                                                                                     |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------- |
| The session stops belonging to a task                  | `require_published` panics on the next EL0 entry; the hardware stamp asserts zero panics |
| The driver loop grows a switch under a mask            | `make irq-scope`                                                                         |
| A document starts using "agent" for the EL0 half alone | **Nothing.** This is prose, and prose stays prose — named here rather than implied       |

## Alternatives rejected

| Alternative                                                    | Why not                                                                                                                                                                                                     |
| -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Collapse the pair: make the EL0 context the schedulable entity | The right long-term shape, and a different kernel. It re-decides four accepted ADRs, and it should be proposed on its own merits when something needs it — not as a side effect of writing down what exists |
| Leave it unrecorded                                            | It is already unrecorded, and that is how `MAX_TASKS`, the fault policy's referent and the true cost of preemption came to look like three separate topics                                                  |
| Record it in `architecture.md` only                            | Descriptive docs say what the code does; this is a decision to _keep_ a shape and to constrain how future ADRs talk about it. That is an ADR's job, and `architecture.md` gets the summary                  |
| Fix the cost first (a smaller driver stack, a shared loop)     | An optimisation argued against no stated baseline. The baseline is what this ADR is                                                                                                                         |
