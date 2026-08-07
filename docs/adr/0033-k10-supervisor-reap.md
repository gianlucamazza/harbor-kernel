---
id: 0033
title: K10 first slice — supervisor reaps a blocked task (and may restart by re-spawn)
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0018, 0023, 0025, 0026, 0031]
---

# ADR-0033: Supervisor reaps a blocked wait (K10 entry)

## Acceptance status

**Accepted** (2026-08-07). First slice of completeness track **K10**: a creator
has a **named product API** to abort a blocked child and reclaim its wait, then
may **restart** by spawning a replacement after the slot is free — without
reboot and without ad-hoc demo-only cancel calls.

Builds on [ADR-0018](0018-agent-fault-policy.md) (creator decides fate) and
[ADR-0025](0025-cancel-blocked-wait.md) (`cancel_blocked` mechanism).

## Context

ADR-0018 left restart/reap as debt: cancel exists, but product code still
wires demos by hand. SECURITY residual “creator lifecycle” names K10.

True remote kill of a **Running** task with a live EL0 session and stack-owned
`AddressSpace` would require AS ownership in the TCB (larger change). This
slice stays honest: **reap a Blocked wait**, then the task’s entry returns and
the trampoline exits (stack/AS cleanup stays the task’s, as today).

## Decision

### 1. `sched::supervisor_reap_blocked(id)`

Trusted EL1 creator path:

1. Refuse idle / unknown / not `Blocked`.
2. Call the same path as `ipc::cancel_blocked(id)` (cancel flag + wake + clear
   waiter).
3. Count `reap_events` (distinct from raw `cancel_events` only if useful; may
   share cancel and add a reap counter for oracles).

Semantics: the child resumes, observes `RecvError::Cancelled` / `Status::Cancelled`,
and **must exit** (return from entry or call `exit`). Product agents treat
Cancelled as terminal for that wait lifecycle.

### 2. Restart = re-spawn after Empty

No in-place “reset TCB and re-enter”. After the reaped task’s slot is
`State::Empty` (exit collected), the creator may `spawn_*` again with the same
or new grants. That is the restart policy for this slice.

### 3. Observation

`sched::task_state(id)` exposes `Option<State>` for creators (not agents).

### 4. Non-goals of this ADR

- Force-kill a **Running** task mid-EL0 without cooperation.
- Store `AddressSpace` in the TCB for remote destroy.
- Creator-exit cascading kill of children (still residual).
- EL0 syscall to kill peers.
- Multi-level supervisor hierarchy.

## Consequences

### Positive

- Named product API over the cancel mechanism.
- Oracle can show reap + restart without reboot.
- Aligns with ADR-0018: kernel remains mechanism; creator decides.

### Costs

- Running agents still require cooperative SessionEnd handling to free AS.
- Second park after Cancelled without exit still leaks a slot (creator bug).

## Gates

| Check | Evidence |
| --- | --- |
| Reap blocked waiter | QEMU `supervisor: reaped id=` |
| Slot free then re-spawn | QEMU `supervisor: restarted id=` |
| Console still up | existing console-server lines |
| Host: reap refuses Ready/Idle | unit or thin sched test if pure path exists |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Remote kill Running without AS-in-TCB | Frame leak or unsafe destroy |
| Auto-restart inside kernel | Policy belongs to creator (0018) |
| EL0 kill syscall | Ambient authority risk |
