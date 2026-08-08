---
id: 0052
title: P5 residual — resolve grant (non-ambient SYS_RESOLVE)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0035, 0039, 0017, 0049]
---

# ADR-0052: Resolve grant (P5 residual)

## Acceptance status

**Accepted** (2026-08-08). Residual of **P5**: `SYS_RESOLVE` is no longer
ambient. A task may resolve a published name only if its creator (or trusted
EL1) has **granted** the resolve right.

## Context

[ADR-0039](0039-p5-el0-resolve.md) allowed any agent to resolve any bound name
("creators control what names exist"). That is fine for lab discovery; production
authority wants resolve itself to be a grant, not ambient ambient lookup.

[ADR-0049](0049-deferred-residuals.md) deferred "name-cap or slot policy". This
slice chooses **slot policy on the TCB**: a boolean grant, not a new CapId kind
(IRQ-cap style minting can supersede later if name service becomes an agent).

## Decision

### 1. Per-task `may_resolve`

- Default **false** on spawn.
- Trusted EL1: `sched::grant_resolve(TaskId)` / `grant_resolve_current()`.
- `SYS_RESOLVE` checks the running task's flag **before** name lookup.
- Refusal without grant → `Status::Authority` (same class as missing name).

### 2. Who grants

Only EL1 creator/bootstrap paths for this slice. No EL0 "grant resolve to peer"
(that would need a task-cap — [ADR-0053](0053-k3-peer-transfer-design.md)).

### 3. Compatibility

Oracles and product agents that call `SYS_RESOLVE` must be granted first.
Missing-name refuse paths still apply after grant.

### 4. Non-goals

- CapId-shaped name-service capability.
- Per-name ACL / namespace isolation.
- Auto-grant from holding any cap.

## Gates

| Check | Evidence |
| --- | --- |
| Bound name without grant refuses | `resolve-grant: refused` |
| After grant, resolve succeeds | existing `el0-resolve: ok` |
| Missing name still refuses | `el0-resolve: refused` |

## Alternatives rejected

| Alternative | Why not now |
| --- | --- |
| New CapId kind for resolve | Larger ABI; boolean is enough for non-ambient |
| Keep ambient forever | Contradicts production least-privilege bar |
