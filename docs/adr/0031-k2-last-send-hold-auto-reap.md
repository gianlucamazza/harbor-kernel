---
id: 0031
title: K2 first slice — auto-cancel waiter when last SEND hold drops
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0017, 0022, 0025, 0026]
---

# ADR-0031: Last SEND-hold auto-reap (K2 entry)

## Acceptance status

**Accepted** (2026-08-07). First slice of completeness track **K2**: when the
last **TCB-held** SEND capability for a channel disappears, a parked waiter on
that mailbox may be cancelled automatically — **if** the channel opted in.

Builds on [ADR-0025](0025-cancel-blocked-wait.md) (`cancel_wait` +
`RecvError::Cancelled`). Does **not** implement a timeout queue.

## Context

ADR-0025 made orphan parks **abortable** by a supervisor. SECURITY residual
remained: nothing auto-cancels when the last send holder exits; a buggy creator
that never cancels still leaks a `Blocked` slot until reset.

Timeout (option A in ADR-0025) needs a deadline queue and a second leave-Blocked
reason. Auto-reap on last SEND drop (option B) reuses the cancel path and
matches the residual wording.

**Console constraint:** the console server parks forever by design. After the
last console-capable agent exits, **zero** TCBs may hold SEND while the server
stays `Blocked`. Naïve “holders==0 ⇒ cancel” would kill the console.

## Decision

### 1. SEND hold count per mailbox

A **hold** is a live TCB slot containing a `CapId` that looks up as SEND for
that mailbox. Stack copies of `CapId` outside TCBs are not holds.

- `+1` when a task is spawned with that SEND cap in a slot (once per slot).
- `−1` when that task exits and its slots are cleared (once per former slot).

### 2. Channel policy: default stable, opt-in ephemeral

| API | `auto_reap` | Use |
| --- | --- | --- |
| `create_channel()` | **false** | Console, intentional servers |
| `create_channel_ephemeral()` | **true** | Agent mailboxes that should die with last sender |

### 3. Last unhold with auto_reap

When `send_holders` reaches 0 on an auto-reap mailbox and a waiter is set:

1. Clear the waiter slot (same as `clear_waiter`).
2. `prepare_cancel_blocked(waiter)` so `recv` returns `Cancelled`.

Status stays **`Cancelled`** (no new ABI value). Supervisor cancel and auto-reap
share the flag; a separate counter is optional.

### 4. Park with zero holders (ephemeral only)

If `auto_reap && send_holders == 0` at park time, refuse park and surface
`Cancelled` immediately (covers never-installed send without a reaper task).

### 5. Layering

`sched` may call `ipc` on spawn/exit for hold registration (K2). `ipc` already
calls `sched` for wake/cancel. Circular imports stay function-level only.

### 6. Non-goals of this ADR

- Timeout / deadline queue (later K2 slice).
- Endpoint release / generation recycle (K3).
- Forced task exit or frame free (still creator / K10).
- Auto-cancel of IRQ-wait parks (K1 path).
- Cap transfer mid-lifetime (K3); only spawn install + exit drop.

## Consequences

### Positive

- Ephemeral agent channels do not need a dedicated reaper task.
- Console and other intentional servers stay default-safe.
- Reuses ADR-0025 cancel machinery end-to-end.

### Costs

- Hold arithmetic must stay accurate (double-grant double-count).
- `sched` → `ipc` edge is new; enforced allow-list update.

## Gates

| Check | Evidence |
| --- | --- |
| Host: last unhold with auto_reap clears waiter | `kernel_core::ipc` unit tests |
| Host: default channel never auto-cancels | unit test |
| QEMU: receiver parks; sole SEND holder exits → cancel | `ipc: auto-reaped cancelled` |
| QEMU: console still up after agents exit | existing console-server lines |
| Supervisor cancel path still works | keep or thin `cancel_blocked` coverage |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Timeout-only first | Heavier; deferred by ADR-0025 on purpose |
| Always auto-reap | Kills console when last agent exits |
| Permanent kernel-held console SEND | Fake holder; muddies authority story |
