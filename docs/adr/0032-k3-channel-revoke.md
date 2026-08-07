---
id: 0032
title: K3 first slice — channel revoke and generation recycle
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0017, 0025, 0026, 0031]
---

# ADR-0032: Channel revoke (K3 entry)

## Acceptance status

**Accepted** (2026-08-07). First slice of completeness track **K3**: a channel
can **die** without reboot so that a later mint may reuse endpoint indices and
stale `CapId`s fail the generation/`live` check for real.

Does **not** implement EL0 cap transfer between agents (later K3 slice).

## Context

Until now endpoints were never released: `live` stayed true, slots were never
recycled, and the generation field was only exercised by host tests that forged
stale handles ([`model_ipc`](../../crates/kernel-core/tests/model_ipc.rs), unit
tests). SECURITY residual: "endpoint release / generation recycle unexercised
on the product path."

Compositions that only grant at spawn and never revoke cannot reconfigure
authority without reboot — the H1 bar item “move authority.”

## Decision

### 1. `Table::revoke_channel(cap)`

Given any live SEND or RECV end of a channel:

1. Mark **both** endpoints for that mailbox `live = false` (generation retained
   on the dead entry until reuse).
2. Clear the mailbox: `live = false`, drain not required (messages discarded),
   `waiter` taken and returned for cancel, `send_holders = 0`.
3. Return `Ok(Some(waiter))` if a task was parked; caller runs the ADR-0025
   cancel path (flag + wake; waiter already cleared).

Bad/stale/dead cap → `RevokeError::BadCap` and authority refusal count.

### 2. Kernel façades

| API | Hold check | Caller |
| --- | --- | --- |
| `ipc::creator_revoke(cap)` | none (trusted EL1 / bootstrap mint still on stack) | bootstrap oracle, future supervisor |
| `ipc::revoke_held(cap)` | `current_holds(cap)` | agent / driver path |

Both end at the same table op. Creator path is not ambient authority for EL0:
EL0 never receives a raw `CapId`.

### 3. Non-goals of this ADR

- Moving a cap from one TCB slot to another (transfer).
- Partial revoke of only SEND or only RECV while leaving the peer live.
- Automatic revoke on last hold (that is K2 ephemeral cancel of *waiters*, not
  endpoint death).
- EL0 syscall for revoke (creator/EL1 first).

## Consequences

### Positive

- Product path can mint → use → revoke → mint; stale handles refuse.
- `!mbox.live` / dead-endpoint arms become reachable without synthetic tables.
- Unblocks later transfer (K3 remainder) and naming (P5) designs.

### Costs

- Messages still queued at revoke are dropped (no flush protocol this slice).
- TCBs may still *hold* CapIds that no longer resolve (send/recv → BadCap).

## Gates

| Check | Evidence |
| --- | --- |
| Host: revoke then stale send is BadCap | unit test |
| Host: revoke frees slots; second create may reuse index with new gen | unit test |
| Host: revoke of parked channel returns waiter | unit test |
| QEMU: `ipc: release stale refused` | boot-check |
| `make doc-claims` / SECURITY residual updated | same PR |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Transfer-only first | Release is what makes generation real; transfer alone leaves permanent live ends |
| EL0 revoke syscall first | No agent product path needs it before creator policy |
| Kill only one end | Leaves half-channel; composition story wants the channel dead |
