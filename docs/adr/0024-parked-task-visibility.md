---
id: 0024
title: Parked tasks are counted; reclaim and timeout remain deliberate non-goals for now
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0006, 0018, 0022, 0023]
---

# ADR-0024: Visibility for parked tasks (issue #13 phase 1)

## Acceptance status

**Accepted** (2026-08-07) by the project owner. Closes the *silent* half of
[#13](https://github.com/gianlucamazza/harbor-kernel/issues/13) without claiming
to fix availability. Reclaim and timeout need a successor ADR (and likely more
code) if the lab starts depending on long-lived parked agents beyond intentional
servers.

## Context

[ADR-0022](0022-blocking-recv-and-the-mask-that-travels.md) made `SYS_RECV` park
the calling **driver task** ([ADR-0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md)).
That park has no timeout and no reclaim. [`SECURITY.md`](../../SECURITY.md) names
the residual: a task waiting on an endpoint whose send end nobody holds keeps
its slot, stack, AS and frames until reset, and until this ADR **no counter
reported it**.

Four shapes were weighed (issue #13):

| Shape | What it needs |
| ----- | ------------- |
| **A** Timeout + deadline queue | Second reason to leave `Blocked`; tick path |
| **B** Creator-side reaping | Policy on the **driver** (ADR-0018 / 0023); does not help if the creator is gone |
| **C** Endpoint release | Generation recycle; wake-with-refusal; large |
| **D** Reporting only | Count tasks in `Blocked`; host-tested |

M8 also parks the **EL1 console server** on an empty mailbox forever by design.
Any global count includes that intentional waiter; the residual risk is
**orphaned** parks (no live send capability), not "something is Blocked".

## Decision

**1. Phase 1 lands reporting (D).**

`kernel_core::tasks::Tasks` exposes:

- `blocked_count()` — how many slots are currently `State::Blocked`
- `block_events()` — how many times a task successfully entered `Blocked` via
  `Switch::Block` since boot (or table construction)

`src/sched` publishes them for voluntary-path readers (idle, demos, diagnostics).
They do **not** run on the IRQ path.

**2. Reclaim and timeout are non-goals of this ADR.**

No deadline queue. No creator reap API. No endpoint release. Those remain open
under issue #13 as **phase 2+** and need their own ADR before code.

**3. The SECURITY residual is updated, not deleted.**

The residual becomes: parks are **visible** via counters; nothing still reclaims
an orphaned waiter; `MAX_TASKS` still bounds how many such leaks fit.

**4. The console server is expected to appear in `blocked_count`.**

After product/oracle boot, at least the console server is typically `Blocked`
while the mailbox is empty. A zero count on a live system that has spawned the
server would be the surprising number — not a non-zero one.

## Consequences

### Positive

- Capacity pressure from parks is greppable and host-testable.
- "Silent forever" stops being true for the *existence* of parks.
- Phase 2 ADRs start from measured baseline, not anecdote.

### Negative / debt

- **Does not free slots or frames.** Availability is unchanged.
- **Does not distinguish orphaned parks from healthy servers** without more
  bookkeeping (why blocked / which endpoint).
- **Creator reaping (B)** and **timeout (A)** remain unscheduled.

### Gates that catch reversal

| Reversal | Gate |
| -------- | ---- |
| Counters removed or always zero while a known block exists | Host tests in `kernel_core::tasks` |
| Count bumps on idle block attempt | Host test: idle `Switch::Block` → `Stay`, no event |
| Claim "parked forever fixed" without reclaim | SECURITY residual still present; doc-claims cannot see prose lies — review |

## Alternatives rejected

| Alternative | Why not |
| ----------- | ------- |
| Implement timeout (A) now | Correct long-term for autonomous agents; changes scheduler contract; ADR-0022 deferred it for a reason |
| Creator reaping (B) now | Right policy direction with ADR-0018/0023, but needs an API and oracle before a counter-less silent leak is solved; do reporting first |
| Endpoint release (C) as the #13 fix | Too large; tracks under generation residual separately |
| Risk-accept with no counter (E) | Leaves the residual strictly worse than D for one afternoon of work |
| Count only "orphans" | Needs endpoint holder graph; not available without C |
