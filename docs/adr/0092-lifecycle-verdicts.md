---
id: 0092
title: Supervisor lifecycle verdicts as a pure decision
status: accepted
date: 2026-08-11
accepted: 2026-08-11
related: [0025, 0033, 0038, 0049, 0058, 0062, 0063, 0090]
---

# ADR-0092: Supervisor lifecycle verdicts as a pure decision

## Acceptance status

**Accepted** (2026-08-11), on delegated authority (structural improvement plan
approved by the owner on 2026-08-11; owner delegated acceptance for the slices
that plan names).

## Problem

Three supervisor entry points in `src/sched/mod.rs` answer the same question —
_given who the target is and what state it is in, what may a supervisor do to
it?_ — and they answer it in three different orders:

|                       | `prepare_cancel_blocked` (ADR-0025) | `supervisor_reap_blocked` (ADR-0033) | `supervisor_force_exit` (ADR-0090) |
| --------------------- | ----------------------------------- | ------------------------------------ | ---------------------------------- |
| idle                  | `false` (via state)                 | `Idle`                               | `Idle`                             |
| unknown / stale epoch | `false`                             | `BadId`                              | `BadId`                            |
| `Empty`               | `false`                             | **`NotBlocked`**                     | **`Empty`**                        |
| `Ready` / `Running`   | `false`                             | `NotBlocked`                         | act (flag + nudge)                 |
| `Blocked`             | mark + wake                         | cancel                               | act (flag + cancel)                |

`Empty` is `NotBlocked` to one caller and `Empty` to the other. That is an ABI
difference — ADR-0061 maps these refusal classes to detail codes an EL0 agent
reads — and today it is guaranteed by nothing but three functions being read
side by side. No host test covers the table, and no mutant can reach it:
mutation testing runs on `kernel-core`, and this decision lives in the kernel
binary. It is the gap [ADR-0058](0058-adr-amendments-and-mutation-freshness.md)
§2 exists to close, and the last of the four **R1 extractions**
[ADR-0049](0049-deferred-residuals.md) has been carrying.

## Decision

`kernel_core::lifecycle` owns the three verdicts, as functions of two scalars:

```rust
pub fn reap(is_idle: bool, state: Option<State>) -> ReapVerdict;
pub fn force(is_idle: bool, state: Option<State>) -> ForceVerdict;
pub fn cancel(state: Option<State>) -> CancelVerdict;
```

The verdict says _what to do_, never _how_: `ReapVerdict::Cancel`,
`ForceVerdict::{FlagAndCancel, FlagAndNudge}`, `CancelVerdict::MarkAndWake`, or
`Refuse(e)` carrying the refusal class. `ReapError` and `ForceError` move here
with them; `sched` keeps its own names as re-exports so the public API and the
ADR-0061 detail codes are untouched.

The kernel keeps everything a pure function cannot know: the lock, writing
`cancel_wait` / `force_exit` into the TCB, `tasks.wake`, the event counters,
`request_resched` + the cross-core SGI, and the call into `ipc::cancel_blocked`
that stays outside `sched` for layering. Each entry point becomes a `match` on
a verdict — mechanism, no decision.

**The `Empty` divergence is kept, not fixed.** Reap answers `NotBlocked`
because reap is _about_ blockedness — an empty slot is simply not blocked, and
that is what its ABI has always said. Force answers `Empty` because force is
about the slot itself, and "there is nothing here" is a different fact from
"it is not waiting". Changing either would be an ABI change with no product
asking for one. What this ADR buys is that the divergence is now **asserted by
a test that names it** rather than implied by reading order.

## Gates

| Check                           | Evidence                                                                                                         |
| ------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Every refusal class host-tested | the full `{idle} × {None, Empty, Ready, Running, Blocked}` table, per verdict, in `kernel_core::lifecycle` tests |
| The divergence is deliberate    | a test asserting `reap(Empty) == NotBlocked` **and** `force(Empty) == Empty` together, with the reason           |
| The decision is mutated         | `lifecycle.rs` in `run-mutants.sh` FILES in the commit it is born (ADR-0058 §2)                                  |
| Behaviour unmoved end to end    | boot oracle reap / cascade / force-exit assertions stay green unchanged                                          |

## Amends ADR-0049

The R1 row lists four extractions. Two were delivered without the row being
reconciled — _sched cap-slot table_ by [ADR-0063](0063-capslots-extraction.md)
and _agent reply mappers_ by ADR-0060. This ADR delivers the third
(_park/cancel composition_). The row is amended to name what actually remains:
**loader plan**.
