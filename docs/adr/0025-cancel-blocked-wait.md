---
id: 0025
title: Cancel a blocked wait — supervisor reaping without a timeout queue
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0018, 0022, 0023, 0024]
---

# ADR-0025: `cancel_blocked` reaps a parked driver task's wait

## Acceptance status

**Accepted** (2026-08-07). Implements issue #13 **phase 2** as creator/supervisor
reaping (option B), not a deadline queue (option A). Builds on
[ADR-0024](0024-parked-task-visibility.md) visibility.

## Context

A task in `State::Blocked` on `ipc::recv` stays there until a send wakes it.
If nobody holds the send end, it waits until reset. Phase 1 made that visible;
this ADR makes it **abortable**.

[ADR-0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md): what is blocked
is the **driver task**. Cancelling the wait resumes that task; it is not a
separate "kill EL0" operation.

[ADR-0018](0018-agent-fault-policy.md): the creator decides the task's fate. This
ADR supplies a mechanism a creator (or other supervisor) can use: abort the
wait so the driver can exit and destroy the address space.

Timeout (A) still needs a deadline queue and a second leave-`Blocked` reason; it
is **not** chosen here.

## Decision

**1. `ipc::cancel_blocked(id)` is the reaping API.**

Steps (voluntary path only):

1. `sched::prepare_cancel_blocked(id)` — require `Blocked`, set per-TCB
   `cancel_wait`, `wake` to Ready, count `cancel_events`.
2. `ipc::clear_waiter(id)` — drop any mailbox waiter for `id` so a later send
   does not invent a wake for a reaped waiter.

**2. `ipc::recv` observes cancel and returns `RecvError::Cancelled`.**

Checked before park and after `block_current`. EL0 maps that to
`Status::Cancelled` (imm reply value 5) without counting an authority refusal.

**3. No automatic cancel on peer exit.**

Dropping the last send capability does **not** auto-wake waiters. That would be
endpoint release / generation policy (option C). Creators must cancel
explicitly — or leave the residual named in SECURITY.

**4. Intentional servers are not special-cased.**

The console server parks forever by design. Nothing cancels it unless a
supervisor calls `cancel_blocked` on its id (which product code must not do
casually).

## Consequences

### Positive

- An orphaned park is recoverable without reboot.
- Oracle proves the path: spawn recv-only, drop send, cancel, see
  `ipc: reaped cancelled`.
- Stays cooperative; no tick-driven deadline queue.

### Negative / debt

- **Still no timeout.** A forgotten cancel is the same leak as before.
- **Still no auto-reap on sender death.**
- **EL0 programs** must handle `Status::Cancelled` or they may treat it as a
  soft failure and loop; demos that care exit after cancel.
- **Frames free only when the task exits and destroys its AS** — cancel only
  unblocks the wait.

### Gates

| Reversal | Gate |
| -------- | ---- |
| Cancel does not clear waiter | Host test: send after clear returns no wake |
| Cancel without cancel flag re-parks forever | Boot-check: `ipc: reaped cancelled` |
| Authority counter inflated by cancel | Cancel maps to `Status::Cancelled`, not Authority |

## Alternatives rejected

| Alternative | Why not |
| ----------- | ------- |
| Timeout queue (A) | Correct for autonomous agents; larger scheduler change |
| Auto-cancel when last send is dropped | Needs holder tracking / release (C) |
| Kill task without waking (force Exit while Blocked) | Leaves waiter slots and EL0 session half-torn; cancel-then-exit is safer |
| Only EL1 cancel, no EL0 status | Hides the event from slot-ABI agents |
