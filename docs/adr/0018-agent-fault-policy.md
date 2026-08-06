---
id: 0018
title: Agent fault policy — the kernel ends the session, the creator decides the task
status: proposed
date: 2026-08-06
---

# ADR-0018: Agent fault policy

## Acceptance status

**Proposed** (2026-08-06). The decision below was taken by the project owner;
the acceptance stamp is a human's and this ADR does not assume it.

Required before M7 by [ADR-0001](0001-multi-role-analysis.md).
[ADR-0016](0016-el0-session-protocol.md) names this ADR as one of its two
successors, and [ADR-0017](0017-el0-capability-abi.md) is the other.

## Context

`SessionEnd::Fault { esr, far }` exists and is returned to whoever ran the
session. Getting there was itself a fix: a faulting agent used to be
indistinguishable from a clean exit, because both returned `Ok(stats)` and the
`ESR`/`FAR` were dropped on the floor. The type's own doc-comment records what
was left open:

> This does not decide what to _do_ about a fault — who kills the agent, who
> restarts it, what is counted — which is the agent fault policy ADR-0016 names
> as missing.

That is the gap. Nothing today counts faults, nothing kills a faulting agent,
and nothing distinguishes — for any observer other than the immediate caller —
an agent that exited from an agent that died.

M7 makes it urgent rather than theoretical. Its done-when has an agent fault
while another keeps running, and _"the supervisor kills it and counts the
fault"_ names a component that does not exist.

## Decision

**The kernel ends the session. The creator decides the task. These are different
questions and conflating them is the error to avoid.**

### 1. Ending the session is mechanism, and the kernel does it unconditionally

An EL0 context that has taken a synchronous exception cannot be resumed safely,
and the kernel is the only party in a position to know that. So on a fault the
kernel ends the session, tears down the EL0 state, and returns
`SessionEnd::Fault { esr, far }`. There is no policy in this and nothing to
configure: it is the same class of decision as _an unknown SVC ends the
session_ ([ADR-0016](0016-el0-session-protocol.md) §6).

### 2. Deciding the task's fate is policy, and the kernel does not make it

Whether the task is killed, restarted, or left to try again is **not** the
kernel's call. `agent::run_user_prog_resuming` already returns `SessionEnd` to
its caller; this ADR promotes that from an accident of how the function was
written into the invariant the design rests on.

The creator decides because the creator already holds the authority: it
allocated the address space, it granted the capabilities
([ADR-0017](0017-el0-capability-abi.md) §2), and it is the only party that knows
what the agent was for. The authority to kill is the authority to have created —
which stays coherent if the creator is one day itself an agent, with no change
to the model.

### 3. The kernel counts, because a counter is mechanism too

A per-task fault count and a machine-wide total, reported in the idle loop
beside the existing refusal counts. The kernel does not act on them. They exist
so that a fault is visible to something other than the immediate caller — the
boot oracle, chiefly, which is how this project verifies anything.

This mirrors `sched::pending_overwrites()`, added for the same reason: an
invariant documented as true and not verified became a counter the idle loop
prints, instead of a comment asserting it.

### 4. No supervisor task, and no `SessionEnd` swallowed

There is no dedicated supervisor process. The creator is the supervisor, and a
caller that ignores a `SessionEnd::Fault` is the bug — so `SessionEnd` is
`#[must_use]`, which turns ignoring one into a compiler warning and, under
`-D warnings`, a build failure.

## Consequences

### Positive

- The kernel stays mechanism. The one decision it makes — the session is over —
  is the one it alone can make correctly.
- No new always-live component. M7 gets a fault policy without gaining a
  process.
- The model does not change when the creator becomes an agent rather than a
  bootstrap function.
- A fault is countable, hence assertable by `boot-check`, hence verifiable in
  the only way this project accepts.

### Negative / debt

- **A creator that exits leaves its agents unsupervised.** Nothing reaps them,
  because nothing owns them once the creator is gone. Today every creator is a
  bootstrap function that outlives everything, so the case does not arise —
  which is exactly the kind of "does not arise yet" this project has been caught
  by before (see the four defects of 2026-08-05, three of which were latent).
- **A restart policy does not exist.** The creator _may_ restart an agent; no
  mechanism helps it, and re-entering an address space whose state caused a
  fault is unexamined territory.
- **`ESR`/`FAR` are reported, not classified.** A stack overflow, a null
  dereference, and a jump into a non-executable page all arrive as
  `SessionEnd::Fault`. The distinction is in the `ESR` and nothing decodes it.
- **A fault in the _creator_ is out of scope.** This ADR covers an agent
  faulting at EL0. An EL1 task taking a fault is a different question with a
  different answer, and this ADR does not pretend to have it.

### Gates that catch reversal

| Reversal                                        | Gate                                                                                                 |
| ----------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| The kernel starts killing tasks on fault        | `make boot-check`: the M7 line where the creator handles a fault and keeps the task alive disappears |
| A fault silently resumes the session            | `boot-check`: the fault count stays at zero while the injected fault happens                         |
| `SessionEnd` ignored at a call site             | `cargo clippy -D warnings` — `#[must_use]`                                                           |
| The fault counter stops counting                | Host test in `kernel-core`, and the `boot-check` assertion on the printed count                      |
| A faulting agent's session state is left behind | Nothing yet, until the M7 slice that adds the assertion on `CURRENT_TCB` (see ADR-0017)              |

Four of five have a gate, and the fifth is the same open row ADR-0017 carries.
That is deliberate: both ADRs depend on one unverified invariant, and it is
better to name it twice than to let each assume the other covers it.

## Alternatives rejected

| Alternative                                    | Why not                                                                                                                                                                                |
| ---------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| The kernel kills the task and counts           | Simplest to verify, and it cables a policy into the kernel — precisely what a microkernel exists not to do. It also forecloses restart, which a supervising agent will eventually want |
| A dedicated EL1 supervisor task                | Closer to a mature agent system, and premature: a new always-live component, a new IPC path, and a second place where authority lives, none of which M7 needs                          |
| Panic on an EL0 fault                          | Punishes the kernel for the agent's mistake. Same reasoning that rejected panicking on an unknown SVC (ADR-0016 §6) — an agent is not trusted input                                    |
| Resume the session after a fault               | An EL0 context that faulted has no defined continuation. Resuming would be inventing one, and the ESR that says why is exactly the evidence that it should not be                      |
| Return `Result` instead of `SessionEnd::Fault` | A fault is an outcome of a session that ran, not a failure to run one. Collapsing it into `Err` loses the stats the session accumulated before it died                                 |

## Related

- [0016](0016-el0-session-protocol.md) — names this ADR as missing; §6 is the
  precedent for "the kernel ends the session and does not judge"
- [0017](0017-el0-capability-abi.md) — the per-task session state that makes a
  per-task fault count meaningful, and the creator's grant that makes it the
  right party to decide
- [0008](0008-irq-handler-policy.md) — the other place this kernel refuses to
  act from a context that cannot act correctly
