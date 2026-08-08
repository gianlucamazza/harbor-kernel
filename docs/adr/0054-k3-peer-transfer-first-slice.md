---
id: 0054
title: K3 first slice — peer transfer via task-cap
status: accepted
date: 2026-08-08
accepted: 2026-08-08
amended: 2026-08-08
related: [0017, 0037, 0041, 0053]
---

# ADR-0054: Peer transfer first slice (K3 residual)

## Acceptance status

**Accepted** (2026-08-08). Implements the first code slice of
[ADR-0053](0053-k3-peer-transfer-design.md): pure task-cap table, mint/lookup,
`SYS_TRANSFER` dest mode 2, revoke-on-exit, dual EL0 oracle.

## Decision

### 1. Pure `kernel_core::taskcap`

- Index band `0x4000 | local` (distinct from IPC and IRQ `0x8000`).
- `mint(task_id)`, `lookup(cap) → task_id`, `revoke_task(task_id)`.
- Host-tested.

### 2. Kernel

- `taskcap::{mint, lookup, revoke_task}` IRQ-masked owner.
- Layering: enforced allow-list gains `sched→taskcap`, `bootstrap→taskcap`,
  `taskcap→arch` (amended 2026-08-08 — the clause was omitted at acceptance;
  ADR-0031 §5 is the template).
- On task exit: `revoke_task(exiting)` so held task-caps go stale.
- `sched::transfer_held_to_peer(from, to_slot, task_cap_slot)`.

### 3. EL0 ABI

`SYS_TRANSFER` with `x2 = 2` (peer): `x3` = local slot holding the destination
task-cap. Self (`0`) and creator (`1`) unchanged ([ADR-0041](0041-el0-cap-transfer.md)).

### 4. Oracle

- Donor holds SEND + task-cap of recipient; EL0 peer-transfers SEND → recipient
  slot 0 → `el0-xfer-peer: ok`.
- Mode 2 without a valid task-cap → `el0-xfer-peer: refused`.

### 5. Residuals

- Auto-mint into creator slot at spawn (EL1 `mint` is explicit for this slice).
- Force-kill / control rights on task-cap.
- Delegation / attenuation — **refused by band since
  [ADR-0055](0055-transferable-cap-bands.md)**; lifting the refusal is a
  successor's job (amended 2026-08-08).

## Gates

| Check | Evidence |
| --- | --- |
| Host taskcap table | unit tests |
| QEMU peer deliver | `el0-xfer-peer: ok` |
| QEMU refuse | `el0-xfer-peer: refused` |
