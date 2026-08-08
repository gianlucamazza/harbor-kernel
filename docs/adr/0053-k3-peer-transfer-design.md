---
id: 0053
title: K3 design — peer transfer via task capability (design only)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0017, 0037, 0041, 0049]
---

# ADR-0053: Peer EL0 transfer design (K3 residual — design accepted, code deferred)

## Acceptance status

**Accepted as design** (2026-08-08). Does not implement peer transfer.
[ADR-0041](0041-el0-cap-transfer.md) remains the product path (self / creator).

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

### First implementation slice (future)

1. Pure `taskcap` table (mint / lookup / revoke-on-exit).  
2. Creator auto-mint on spawn into a reserved slot or side channel.  
3. `SYS_TRANSFER` mode 2 wiring.  
4. Oracle: A transfers to B only with B's task-cap.

### Non-goals of this document

- Force-kill / remote AS destroy via task-cap.  
- Delegation chains / attenuation.  
- Implementing peer transfer now.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Raw TaskId in `x2` | Ambient naming of all tasks |
| Global "any transfer" right | Too coarse |

## Deferral

Code waits for CapId namespace discipline (endpoint vs IRQ vs task) to stay
readable; irqcap is the template.
