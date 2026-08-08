---
id: 0053
title: K3 design — peer transfer via task capability (design only)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
amended: 2026-08-08
related: [0017, 0037, 0041, 0049]
---

# ADR-0053: Peer EL0 transfer design (K3 residual)

## Acceptance status

**Accepted as design** (2026-08-08). First code slice landed in
[ADR-0054](0054-k3-peer-transfer-first-slice.md). Self / creator path remains
[ADR-0041](0041-el0-cap-transfer.md).

## Context

EL0 may move a held cap to **self** or **creator**. Peer-by-TaskId was deferred
([ADR-0049](0049-deferred-residuals.md)): naming an arbitrary task from EL0 is
ambient authority.

## Decision (design)

| Item | Intent |
| --- | --- |
| Task capability | At spawn, creator receives a **task-cap** naming the child (generation-stamped id, CapId-shaped or parallel table like irqcap) |
| EL0 peer transfer | `SYS_TRANSFER` dest mode `2` = peer: `x3` = slot holding task-cap of destination; dest slot empty |
| Rights | Holding a task-cap is the right to install caps into that task's empty slots (transfer only — not kill/control yet) |
| Lifecycle | Task-cap invalid on child exit (generation bump); creator may drop without killing child |
| Evidence (when coded) | Host table model; QEMU `el0-xfer-peer: ok` / refuse stale task-cap |

### First implementation slice

Landed in [ADR-0054](0054-k3-peer-transfer-first-slice.md):

1. Pure `taskcap` table (mint / lookup / revoke-on-exit).  
2. EL1 `mint` (explicit; auto-install on spawn is residual).  
3. `SYS_TRANSFER` mode 2 wiring.  
4. Oracle: A transfers to B only with B's task-cap.

### Non-goals of this document

- Force-kill / remote AS destroy via task-cap.  
- Delegation chains / attenuation.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Raw TaskId in `x2` | Ambient naming of all tasks |
| Global "any transfer" right | Too coarse |

## Deferral

Further depth (auto-mint on spawn, control rights) waits for product need.

**Amended 2026-08-08** (ADR-0058 convention): reconciled with the landed first
slice — [ADR-0054](0054-k3-peer-transfer-first-slice.md), commit `0cee6e4`.
Mint is explicit EL1; auto-mint on spawn demoted to residual. The evidence row
"refuse stale task-cap" is now honoured end to end by `xfer-peer: stale
refused` ([ADR-0057](0057-taskcap-lifecycle.md)).
