---
id: 0049
title: Deferred residuals — peer EL0 transfer, resolve-grant, P3/P4 without composition
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0026, 0039, 0041, 0060, 0063, 0092]
amended: 2026-08-17
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
| **P4 product display** | No product UI composition. The lab panel that stood in for one is **retired** ([ADR-0094](0094-retire-debug-display.md), 2026-08-11) | A product UI ADR, starting from a composition rather than from that driver |
| ~~**#14 SpiDevice**~~ | **Closed 2026-08-11**: ADR-0020 superseded by [ADR-0094](0094-retire-debug-display.md) — the trait went with the panel. A permanent watch was a retirement nobody had scheduled | — |
| ~~**Panic-path oracle**~~ | **Delivered** by [ADR-0093](0093-panic-path-positive-evidence.md) (2026-08-11): a `panic-probe` image faults on a real stack guard page, and `make panic-check` asserts the whole chain — including that the printed `FAR` is the address the probe announced | — |
| ~~**Task-cap spawn epoch**~~ | **Delivered** by [ADR-0062](0062-taskid-epoch.md) (2026-08-09): the epoch lives in `TaskId` itself and the task-cap entry stores it (`taskcap.rs` raw id), so the exit→revoke window is closed structurally | — |
| **H3 L1+ / x86 EL0 stubs** | `src/arch/x86_64/el0.rs` carries three `panic!("x86 L0: … not implemented")` for `enter`, `resume` and `run`. Consistent with L0 being `done (QEMU-x86)` ([ADR-0071](0071-h3-l0-x86-qemu-first-slice.md)) and L1+ not started — but they were on **no** residual list until now, and a residual nobody lists is one nobody revisits (2026-08-17 review, F-16) | An H3 L1 design ADR, which is what would give them a body. Until then they are the honest shape of an unwritten path: a stub that panics is louder than one that returns a plausible zero |
| ~~**Derived mutation file list**~~ | **Closed 2026-08-17** — the trigger fired: `genet.rs` reached 3142 lines over 51 commits without ever being mutated, and `mutation-freshness` could not see it because it counts only inside the scope. Scope is now `docs/mutation-scope.toml`, every kernel-core module carries a recorded decision (`in_scope` / `queued` / `exempt` with a reason), `make mutation-scope` refuses an unclassified module (seen red on `genet`/`genet_fdt`), and `run-mutants.sh` derives its `--file` list from that same file instead of keeping a second copy | — |
| ~~**kernel-core extractions**~~ | **Delivered in full.** The loader plan, the last of the four, by [ADR-0097](0097-loader-plan.md) (2026-08-11); the others by ADR-0063, ADR-0060 and [ADR-0092](0092-lifecycle-verdicts.md). What was left outside the host-test and mutation nets in `src/` was described here as mechanism — MMIO, assembly, lock discipline — not decisions. **Amended 2026-08-17:** that is no longer true. `src/drivers/genet.rs` is 817 lines of decisions — init ordering, when to assert `RGMII_LINK`, when to arm `RX_EN`, what `boot_after_program` sequences, which settle is acceptable — and none of it is tested ([2026-08-17 review](../reviews/2026-08-17-excellence.md) F-11). The extractions are still delivered; the claim about what remains is retired | — |

> **Amendment (2026-08-11, reconciliation per [ADR-0058](0058-adr-amendments-and-mutation-freshness.md) —
> reconciled by [ADR-0092](0092-lifecycle-verdicts.md)).** The R1 row named four
> extractions and was never narrowed as they landed. Three are delivered: *sched
> cap-slot table* by [ADR-0063](0063-capslots-extraction.md), *agent reply
> mappers* by ADR-0060, and *sched park/cancel composition* by ADR-0092
> (`kernel_core::lifecycle`). Only the **loader plan** remains, and the row now
> says so. A residual list that keeps naming paid work is a list nobody trusts
> to name the unpaid.

## Not deferred (landed elsewhere)

K5 thin stacks, P2 durable region, K4 cooperative budget, K7 first slice, K4/K8
preemption·SMP design ADRs, **resolve grant** ([ADR-0052](0052-p5-resolve-grant.md)).
