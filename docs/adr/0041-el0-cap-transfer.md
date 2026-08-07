---
id: 0041
title: K3 residual — EL0 SYS_TRANSFER (self slots or return to creator)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0017, 0037]
---

# ADR-0041: EL0 capability transfer (K3 residual entry)

## Acceptance status

**Accepted** (2026-08-08). Residual of **K3**: an agent may **move** a held
cap between its own empty slots, or **return** it to an empty slot of its
**creator**, without ever naming a raw `CapId` ([ADR-0017](0017-el0-capability-abi.md)).

Peer-to-peer transfer by TaskId remains deferred (would ambiently name tasks).

## Decision

### `SYS_TRANSFER` = imm 8

| register | meaning |
| --- | --- |
| `x0` | source slot (must hold a cap) |
| `x1` | destination slot (must be empty) |
| `x2` | `0` = self; `1` = creator |
| `x0` out | [`Status`](../../crates/kernel-core/src/syscall.rs) |

Uses the same TCB move as [`transfer_held`](0037-k3-cap-transfer.md) (self path
is same-task; creator path targets `tcb.creator`). SEND-hold counts unchanged.

### Non-goals

- Transfer to arbitrary TaskId / peer agents.
- Copy (duplicate) of a cap.
- Rights attenuation.

## Gates

| Check | Evidence |
| --- | --- |
| Decode(8) | unit test |
| QEMU return-to-creator | `el0-xfer: ok` |
| QEMU refuse bad dest | `el0-xfer: refused` |
