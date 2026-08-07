---
id: 0037
title: K3 residual — EL1 capability transfer between tasks
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0017, 0031, 0032]
---

# ADR-0037: Capability transfer (K3 residual entry)

## Acceptance status

**Accepted** (2026-08-08). Residual of completeness track **K3**: a holder may
**move** a `CapId` from one of its slots into an empty slot of another live task
so authority can change without minting a new channel or rebooting.

Does **not** implement EL0 transfer syscalls or partial rights attenuation.

## Context

[ADR-0032](0032-k3-channel-revoke.md) made channels die and recycle generations.
H1 bar item 6 still needed **move authority** among live agents: grant at spawn
is static; revoke alone cannot hand a live end to a peer.

## Decision

### 1. `sched::transfer_held(from_slot, to_task, to_slot)`

Trusted EL1 (current task is the donor):

1. `from_slot` must hold a `CapId` on the current task.
2. `to_task` must be a live non-empty slot other than self.
3. `to_slot` must be empty and in range.
4. Move the `CapId`: clear donor slot, write recipient slot.

**Hold counts:** SEND-hold counters ([ADR-0031](0031-k2-last-send-hold-auto-reap.md))
count TCB installs. A move preserves the total number of holders, so counters
are **unchanged** (no register/release).

### 2. Non-goals

- EL0 `SYS_TRANSFER`.
- Copy (duplicate) of a cap — only move.
- Rights narrowing on transfer.
- Transfer of IRQ notification caps as a special path (same CapId move works if held).

## Gates

| Check | Evidence |
| --- | --- |
| QEMU donor empties, recipient holds and may send | `ipc: transfer ok` |
| Host: N/A pure (TCB policy) | oracle + refuse paths in demos if needed |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Always revoke+remint | Loses generation continuity and queued messages |
| EL0 first | No product agent needs self-serve transfer before creator policy |
