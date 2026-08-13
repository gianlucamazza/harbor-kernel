---
id: 0090
title: K10 residual — force-exit a Running (or Ready) task at a safe point
status: accepted
date: 2026-08-10
accepted: 2026-08-10
amended: 2026-08-13
related: [0018, 0023, 0025, 0033, 0038, 0064]
---

# ADR-0090: Force-exit Running tasks (K10 residual first slice)

## Acceptance status

**Accepted** (2026-08-10). Closes the K10 residual named in
[ADR-0033](0033-k10-supervisor-reap.md) §4 and the roadmap K10 row:
**force-exit** of a non-Blocked task without requiring a cooperative
`SYS_EXIT` from EL0 text.

Status: **done (QEMU)** via `make boot-check` after land; HW stamp optional.

> **Amendment (2026-08-13, reconciliation per [ADR-0058](0058-adr-amendments-and-mutation-freshness.md)).**
> The optional stamp landed: **done (HW)**, transcript `20260811-122821.log`
> (`force-kill: requested` → `child forced` → `slot empty` on silicon). Living
> status is the roadmap K10 row, not the landing line above.

## Context

| API | Covers |
| --- | --- |
| `supervisor_reap_blocked` (0033) | **Blocked** wait only |
| Creator-exit cascade (0038) | Cancel blocked children |
| **Gap** | **Running** / **Ready** driver with live EL0 session |

ADR-0033 deferred true remote kill mid-EL0 (AS in TCB, cross-core tear-down).
This slice stays honest: the **victim still tears down its own AS** on its
stack after observing a flag at a **safe point** — not a remote `destroy`
from the supervisor.

## Decision

### 1. `sched::supervisor_force_exit(id)`

Trusted EL1 creator path:

| State | Action |
| --- | --- |
| Idle / unknown / Empty | Refuse (`ForceError`) |
| **Blocked** | Set `force_exit` **and** `ipc::cancel_blocked` (same wake as reap) |
| **Ready** / **Running** | Set `force_exit`; if home ≠ local CPU, `request_resched` (+ SGI to CPU1) |

Count `force_exit_events` for oracles.

### 2. Safe points that consume the flag

| Site | Behaviour |
| --- | --- |
| `task_trampoline` before `entry()` | Skip entry; `exit()` immediately |
| Agent session loop (`run_user_prog_resuming*`) top of loop | `end_step`; `SessionEnd::Forced`; return to creator for `destroy` |
| EL1 body (optional) | Poll `take_force_exit()` in long workers |

**Not** a safe point in this slice: arbitrary PC inside EL0 with IRQs
masked forever. Product agents with IRQs open (or Blocked) are covered;
a hard-spin masked EL0 remains residual (same class as “no force mid-instruction”).

### 3. `SessionEnd::Forced`

New terminal outcome: supervisor requested stop; session ended by kernel
mechanism; **creator still decides** task fate (destroy / restart) —
ADR-0018 unchanged.

### 4. Non-goals

- Remote AS destroy / AS ownership in TCB  
- EL0 syscall to kill peers  
- Guaranteed kill of IRQ-masked infinite EL0 spin without preemption  
- Superseding pair shape (K5-B)  

## Gates

| Check | Evidence |
| --- | --- |
| Host | N/A (sched flag is kernel crate) |
| QEMU | `force-kill: requested`, `force-kill: child forced`, `force-kill: slot empty` (EL1 Running oracle; agent loop also ends on `SessionEnd::Forced`) |
| Docs | roadmap K10 residual closed for this slice; verification row |

## Alternatives rejected

| Option | Why not |
| --- | --- |
| Only document residual forever | Named completeness debt with no path |
| Full remote kill + AS in TCB | Larger design; defer as K10-R if product needs it |
| Reuse `cancel_wait` alone | Semantics are park-cancel, not “exit the agent” |

## Related

- Reap blocked: [0033](0033-k10-supervisor-reap.md)  
- Fault policy: [0018](0018-agent-fault-policy.md)  
- Cancel: [0025](0025-cancel-blocked-wait.md)  
