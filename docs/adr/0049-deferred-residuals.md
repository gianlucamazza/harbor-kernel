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
| **Peer transfer residuals** | First slice **done (QEMU)** ([ADR-0054](0054-k3-peer-transfer-first-slice.md)); auto-mint on spawn / control rights open | Product need |
| **P3 network** | No named composition target this cycle | Edge-gateway composition + virtio/net ADR |
| **P4 product display** | `debug-display` remains lab; no product UI composition | Product UI ADR graduating debug-display |
| **#14 SpiDevice** | Watch only; XPT2046 not in scope | Touch driver or supersede ADR-0020 |
| **Panic-path oracle** | `src/panic.rs` has negative evidence only (excellence review F-24); a deliberate-panic boot variant is a new image flavour | Next bring-up/oracle work touching the panic path |
| ~~**Task-cap spawn epoch**~~ | **Delivered** by [ADR-0062](0062-taskid-epoch.md) (2026-08-09): the epoch lives in `TaskId` itself and the task-cap entry stores it (`taskcap.rs` raw id), so the exit→revoke window is closed structurally | — |
| **Derived mutation file list** | `run-mutants.sh` list is hand-written; scope now validated but membership is still a decision ([ADR-0058](0058-adr-amendments-and-mutation-freshness.md)) | Marker-derived list, or the next membership miss |
| **kernel-core extractions** | sched cap-slot table, agent reply mappers, loader plan, sched park/cancel composition (excellence review R1) — highest-value path to host-testing `src/` logic | Next slice touching each module |

## Not deferred (landed elsewhere)

K5 thin stacks, P2 durable region, K4 cooperative budget, K7 first slice, K4/K8
preemption·SMP design ADRs, **resolve grant** ([ADR-0052](0052-p5-resolve-grant.md)).
