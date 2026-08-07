---
id: 0040
title: K2 residual — park timeout on tick deadlines
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0006, 0008, 0025, 0031]
---

# ADR-0040: Park timeout (K2 residual entry)

## Acceptance status

**Accepted** (2026-08-08). Residual of completeness track **K2**: a blocked IPC
wait can **expire** at an absolute monotonic tick deadline and resume with
[`RecvError::Cancelled`](0025-cancel-blocked-wait.md) — same cancel machinery as
supervisor cancel and last-SEND-hold auto-reap ([ADR-0031](0031-k2-last-send-hold-auto-reap.md)).

## Context

ADR-0031 closed auto-reap when the last sender dies. A waiter with a live but
silent peer (or a forgotten sender) still parks forever. H1 supervise residual
“timeout” names this gap. Full deadline heaps and EL0 ABI are out of scope for
the first slice.

## Decision

### 1. Pure table `kernel_core::parktime`

- Fixed slots: `arm(task, deadline_tick)`, `disarm(task)`, `poll(now) → expired tasks`.
- Absolute deadlines on the same counter as `time::ticks()` (timer IRQ).
- Host-tested.

### 2. Product API `ipc::recv_with_timeout(cap, timeout_ticks)`

1. `deadline = ticks() + max(timeout_ticks, 1)`
2. Arm the current task’s deadline.
3. Call existing blocking [`recv`](0022-blocking-recv-and-the-mask-that-travels.md).
4. Disarm on return (success, cancel, or error).

Expiry uses **`prepare_cancel_blocked` + clear waiter** — no new `Status` value.
Agents treat timeout like supervisor cancel for this slice.

### 3. Poll on the voluntary path

`sched::poll_wakes` (idle loop / yield) calls `parktime::poll` then
`ipc::cancel_blocked` for each expired id. **No context switch from IRQ**
(ADR-0006 / ADR-0008). Timer IRQ only advances ticks; idle’s next
`poll_wakes` after WFI delivers the cancel.

### 4. Non-goals

- EL0 `SYS_RECV` timeout (later).
- Timeout on IRQ wait.
- Distinct `Status::TimedOut` ABI.
- Soft real-time guarantees / priority.

## Gates

| Check | Evidence |
| --- | --- |
| Host arm/poll/disarm | unit tests |
| QEMU timed park without sender | `ipc: timed-out cancelled` |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Cancel from timer IRQ handler | Risk of nested policy; voluntary poll is enough with 10 Hz idle |
| New TimedOut status first | ABI surface; Cancelled already means “wait aborted” |
| Only document “use supervisor” | Silent peers still hang autonomous agents |
