---
id: 0038
title: K10 residual — cascade cancel of blocked children on creator exit
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0018, 0025, 0033]
---

# ADR-0038: Creator-exit cascade (K10 residual entry)

## Acceptance status

**Accepted** (2026-08-08). Residual of completeness track **K10**: when a task
**exits**, every **Blocked** child it created is cancelled (same mechanism as
[`cancel_blocked`](0025-cancel-blocked-wait.md) / supervisor reap), so orphans
do not park forever after the creator is gone.

Does **not** force-kill Running children or destroy their address spaces remotely.

## Context

[ADR-0033](0033-k10-supervisor-reap.md) named creator-exit cascading as residual.
A supervisor that parks children then exits leaves blocked waiters with no one
to reap them — the same leak class as forgotten cancel.

## Decision

### 1. Record creator at spawn

Each TCB stores `creator: TaskId` set to the spawning task (idle for early
bootstrap spawns is fine).

### 2. On `Switch::Exit`

After snapshotting exit caps (K2 hold release), scan live tasks:

- if `tcb.creator == exiting` and task state is `Blocked`, run the cancel path
  (`ipc::cancel_blocked` / prepare_cancel + clear waiter).

Count `cascade_events` for oracles.

### 3. Non-goals

- Kill Ready/Running children.
- Multi-level cascade beyond direct children (grandchildren only if their
  creator is the exiting task).
- Re-parenting.

## Gates

| Check | Evidence |
| --- | --- |
| QEMU child cancelled after parent exit | `cascade: cancelled` |
| Cascade counter | optional `cascade: events=N` |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Only document “creators must reap” | Product path still leaks under exit races |
| Force-kill Running | Needs AS-in-TCB; ADR-0033 deferred |
