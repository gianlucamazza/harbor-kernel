---
id: 0049
title: Deferred residuals — peer EL0 transfer, resolve-grant, P3/P4 without composition
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0026, 0039, 0041]
---

# ADR-0049: Explicit deferrals (policy)

## Acceptance status

**Accepted** (2026-08-08). Records honest **deferrals** so completeness tracking
does not invent product or ambient authority.

## Deferred (with rationale)

| Item | Why deferred | Unblock when |
| --- | --- | --- |
| **Peer EL0 transfer** (TaskId / task-cap) | Naming arbitrary tasks from EL0 is ambient; needs task capabilities | Task-cap design ADR |
| **Resolve grant** (non-ambient SYS_RESOLVE) | Ambient resolve is acceptable for lab; production needs a resolve right | Name-cap or slot policy ADR |
| **P3 network** | No named composition target this cycle | Edge-gateway composition + virtio/net ADR |
| **P4 product display** | `debug-display` remains lab; no product UI composition | Product UI ADR graduating debug-display |
| **#14 SpiDevice** | Watch only; XPT2046 not in scope | Touch driver or supersede ADR-0020 |

## Not deferred (landed elsewhere)

K5 thin stacks, P2 durable region, K4 cooperative budget, K7/K8 design ADRs.
