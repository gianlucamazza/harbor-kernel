---
id: 0080
title: K8 design — EL0 sessions and agents with home on CPU 1
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0014, 0016, 0017, 0019, 0023, 0048, 0050, 0051, 0064, 0068, 0070, 0075, 0076, 0077, 0078, 0079]
---

# ADR-0080: EL0 on CPU 1 (design)

## Acceptance status

**Accepted as design** (2026-08-10). After dual-current + fair EL1 quantum on
CPU 1 (**done (HW)** — [ADR-0076](0076-k8-per-core-queues-first-slice.md)/[0077](0077-smp-shared-state-discipline.md)/[0079](0079-k8-per-core-timer-preemption-first-slice.md)),
product agents and EL0 sessions still **home only on CPU 0**. This ADR is the
design for closing that gap. First **code** is a follow-on ADR (expected
**0081**).

## Context

| Piece | Today |
| --- | --- |
| Sticky hard affinity + `spawn_on` | **done (HW)** |
| EL1 quantum + timer on CPU 1 | **done (HW)** — `preempt-el1-cpu1:*` |
| `publish_el0` on switch | **CPU 0 only** (`if cpu == 0` in `switch_with`) |
| `CURRENT_EL0` | **Single** machine-wide `AtomicPtr` (ADR-0019); asm loads it without affinity |
| `el0::current()` contract | Documents single-core + local IRQ mask |
| Product agents / loader | Always admitted on the spawning core’s default (CPU 0) |
| Console TX from core 1 | **Forbidden** (ADR-0070); agents print via `SYS_SEND` → console server |
| Steal | Out of scope (ADR-0075); still residual |

Without per-CPU published sessions, two concurrent EL0 entries (one per core)
silently share one pointer — the class of bug ADR-0017’s “published session
must be this task’s” check was written to catch on one core.

## Decision

### 1. Per-CPU published EL0 session (mandatory)

| Item | Choice |
| --- | --- |
| Storage | `CURRENT_EL0[N_CPUS]` as `[AtomicPtr<El0Session>; N]` (or equivalent) |
| Index | Affinity level 0 (`arch::cpu::affinity`), same bound as timer/sched mirrors |
| `publish(session)` | Writes **this** core’s slot only |
| Loads (Rust + asm) | Always index by affinity — `vectors.s` lower-EL IRQ/SVC paths included |
| Invariant | Core *i* never reads or writes slot *j* for *i ≠ j* |

**Rationale:** banked TTBR0/ASID already allow two ASes live on two cores;
the published pointer is the only global that still assumes one EL0 at a time.
A single pointer with a “never two EL0s” policy is fragile and blocks product
dual-agent SMP.

ADR-0019’s ordering (Release publish / Acquire load) remains per slot.

### 2. Publish on every switch, every affinity

Remove the `cpu == 0` fence around `publish_el0` in `switch_with`. CPU 1 idle
publishes null (or the idle slot’s empty session) exactly as CPU 0 does.

No special “first EL0 on secondary” bring-up beyond: IRQs unmasked, timer
live (0079), and a task with a real `El0Session` switched in.

### 3. Spawn policy — sticky home, explicit affinity

| Rule | Detail |
| --- | --- |
| Hard sticky home | Unchanged from ADR-0075 — no steal in this design or first code |
| Default product spawn | Remains **CPU 0** unless the creator asks otherwise |
| Explicit pin | `spawn_on(1, …)` / `spawn_on_with_caps(1, …)` (names free in code ADR) admit driver tasks whose entry runs agents on home=1 |
| Loader / product store | First code may keep product agents on 0; lab/oracle pins at least one EL0 session on 1 |

The schedulable entity remains the **EL1 driver** ([ADR-0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md));
home is a property of that task, not of the EL0 program image.

### 4. EL0 preemption on CPU 1 — in scope for first code

| Path | First code slice (~0081) |
| --- | --- |
| EL0 lower-EL quantum (ADR-0064) | **In scope** — agent loop `preempt_switch` uses per-CPU `SLICE_START` / global ticks already (0077/0079) once the driver homes on 1 |
| EL1 same-EL (ADR-0068/0079) | Already paid on CPU 1; no change required for this design |
| Device IRQ handler switch | Still **forbidden** (ADR-0008) |

No new EL0 pivot assembly: the 0064 shape (frame already in `El0Session`,
switch in task context after EOI) is core-agnostic once publish is per-CPU.

### 5. TTBR0 / ASID / TLB

| Concern | Rule for this design |
| --- | --- |
| TTBR0 + CONTEXTIDR on enter/resume | Already per-task; run on whichever core owns the driver |
| Cross-core TLB shootdown | **Out of scope** — sticky home means the AS is exercised on its home core; no remote invalidation required for the first slice |
| TTBR1 high-half | Still K7 residual — not gated by EL0-on-CPU1 |

If a future steal ADR moves a task with a live AS across cores, **that** ADR
owns TLB IPI / ASID rollover policy.

### 6. Console and IPC

| Path | Rule |
| --- | --- |
| Kernel `kprintln` / panic TX from core 1 | Still **out of product path** (ADR-0070) |
| Agent console | `SYS_SEND` to console-server (home 0); wake already routes to home (0075) |
| IPC / park / irq-wait | Existing home-queue + SGI resched; audit call sites in code ADR |

Oracle lines for CPU1 EL0 demos are printed by a **CPU0 watcher** or by the
primary after atomics, same discipline as `preempt-el1-cpu1` (0079).

### 7. Evidence (for the code ADR)

| Claim | Gate (sketch) |
| --- | --- |
| Session published on affinity 1 | Implicit via successful EL0 entry; optional `el0-cpu1: published` |
| EL0 work ran on home=1 | Oracle e.g. `el0-cpu1: svc ok` and/or dual-agent concurrent |
| Non-yielding EL0 spinner on home=1 loses CPU | `preempt-el0-cpu1: rotated` (+ spinner exited) — stronger if first code includes it |
| Primary K4 / K8 oracles unchanged | Existing `preempt:` / `preempt-el1:` / `preempt-el1-cpu1:` / `smp:` lines |
| QEMU then HW | `boot-check`; Pi `hw-transcript-check` |

Exact strings live in the code ADR. Prefer one strong EL0-on-CPU1 claim over
many weak ones.

### 8. First implementation slice (follow-on code ~0081)

Ordered:

1. Per-CPU `CURRENT_EL0` + Rust publish/load + **asm** affinity index  
2. `publish_el0` on all affinities in `switch_with`  
3. Spawn-with-home API for capped / agent drivers on CPU 1  
4. Oracle: EL0 session on home=1 (SVC and/or EL0 preempt pair)  
5. Docs: **done (QEMU)** then HW stamp  
6. MAX_TASKS / stack headroom if concurrent demos require it  

### 9. Explicit non-goals

- Work stealing / soft affinity  
- Cores 2–3  
- Console TX from CPU 1  
- TLB shootdown IPI  
- TTBR1  
- Changing product default home to 1  
- Collapsing the agent driver half (K5 residual)  
- Dual-core global tick producers (0078)

## Alternatives considered

| Alternative | Why not first |
| --- | --- |
| Keep one `CURRENT_EL0`; forbid concurrent dual EL0 | Breaks under product dual-agent; silent failure mode |
| EL1-only on CPU1 forever | Leaves product multi-core incomplete; 0079 already paid EL1 |
| Bundle steal in the same ADR | Separate failure modes; 0075/0048 keep steal later |
| Full EL0+EL1 redesign of vectors | 0064/0068 paths are core-agnostic once publish is fixed |

## Consequences

### Positive

- Product-shaped dual-core: agents may home on either schedulable CPU  
- Completes the 0078 residual “EL0-on-CPU1” honestly  
- Steal becomes a load-balance story instead of a lab-only one  

### Residual after first code

- Steal design/code  
- Cores 2–3  
- Cross-core TLB if steal lands  
- K7 TTBR1 / switch-cost; K5 driver-half  

### Gates (design-level)

| Reversal | Catch |
| --- | --- |
| Global `CURRENT_EL0` left in place | Dual EL0 corruption; missing oracle under concurrent agents |
| Publish still CPU0-only | EL0 enter on CPU1 panics “no published session” |
| Steal smuggled in | Review against §9 |

## Related

- Session / authority: [0016](0016-el0-session-protocol.md), [0017](0017-el0-capability-abi.md), [0019](0019-no-static-mut.md) (`CURRENT_EL0` as `AtomicPtr`)  
- Agent shape: [0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md)  
- K4: [0064](0064-k4-el0-preemption-first-slice.md), [0068](0068-k4-el1-preemption-second-slice.md)  
- K8: [0075](0075-k8-per-core-queues-design.md)–[0079](0079-k8-per-core-timer-preemption-first-slice.md)  
- ASID: [0050](0050-k7-asid-first-slice.md)  
