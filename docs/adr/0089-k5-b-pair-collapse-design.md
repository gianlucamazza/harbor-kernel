---
id: 0089
title: K5-B design — pair collapse (session as schedulable entity)
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0006, 0016, 0017, 0018, 0021, 0022, 0023, 0026, 0044, 0064, 0068, 0080, 0085, 0086]
---

# ADR-0089: K5-B pair collapse (design only)

## Acceptance status

**Accepted as design** (2026-08-10). This ADR **defines** residual **K5-B**
from [ADR-0085](0085-k5-density-residual-design.md) §1 and the shape of record
in [ADR-0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md). It does
**not** implement collapse, does **not** supersede ADR-0023, and does **not**
authorize code.

**Code remains deferred** until:

1. At least one **trigger** in §3 fires (same bar as ADR-0085 §3), **and**
2. A **code-series plan** (§6) is accepted as one or more successor ADRs
   (this document is the policy parent, not a silent green light to rewrite
   `sched` / `agent` / exception paths).

Issue tracker: [#25](https://github.com/gianlucamazza/harbor-kernel/issues/25).

## Context

### What is paid

| Piece | Status |
| --- | --- |
| Pair shape (driver + EL0 program) | **Record** — ADR-0023 |
| K5 thin / Mini stacks | **done (HW)** — ADR-0044 / ADR-0086 |
| K5 residual language K5-S/H/B | **policy** — ADR-0085 |
| Preemption EL0+EL1, dual-current, EL0-on-CPU1, steal | **done (HW)** |
| Product `home_cpu` | **done (QEMU)** — ADR-0088 |
| Slot meter | Still **one driver task per agent** (+ infra) |

### Cost the pair still imposes (ADR-0023 restated)

| Per concurrent agent | Cost |
| --- | --- |
| One `sched` slot | Shares `MAX_TASKS` with idle, console server, oracles |
| One kernel stack | Full / Thin / Mini class (heap meter paid by K5-S) |
| One `El0Session` in the TCB | ADR-0017 |
| One address space | Option C clone (map density → K7-T if trigger) |

**K5-S fixed heap stack.** It did **not** remove the slot or the synchronous
driver loop that sits in a call frame for the whole session. That is the
density frontier multi-role F-R7-2 named.

### Why not implement now

ADR-0023: making the EL0 context the schedulable entity is a **different
kernel**. It reopens cooperative vs preempt models (0006/0064/0068), session
ownership (0016/0017), fault/creator policy (0018), and park/mask (0022).
Doing that for aesthetic “finish K5” fails ADR-0085 §3 (not a trigger).

## Decision

### 1. What K5-B means (normative)

**K5-B — pair collapse** = the **schedulable entity becomes the agent session
(EL0 context + grants + AS)**, not a long-lived EL1 driver task that owns a
synchronous enter/resume loop for the whole session.

Concretely, after a full K5-B land:

| Today (pair) | After K5-B |
| --- | --- |
| `sched` admits a driver `fn()` that runs `Agent::run_user_prog_resuming` | `sched` admits/resumes an **agent identity** whose runnable state is “ready at EL0” or “blocked in kernel on a named wait” |
| One task slot per concurrent agent session | One task/session slot **is** the agent (no second “driver half” for the common case) |
| Exception path returns into the driver loop | Exception path returns into the **scheduler** (or a thin shared trampoline), which picks the next session |
| Creator “kills the task” = kills the driver | Creator policy names the **agent/session** identity |

Plain kernel tasks (console server, idle, short workers) **remain** EL1-only
tasks. They are not agents and do not need a pair.

### 2. What K5-B is not

| Not K5-B | Owned by |
| --- | --- |
| Smaller stacks | K5-S (paid) |
| M drivers multiplexing N sessions | **K5-H** (still deferred; §4) |
| Raising `MAX_TASKS` | Forbidden as density (ADR-0085) |
| TTBR1 / AS frame density | K7-T if trigger (ADR-0084) |
| Linux processes / POSIX | Out of model |

K5-H is a **different** residual: keep the pair shape, share driver tasks.
K5-B **removes** the long-lived per-agent driver. They are alternatives for
the slot meter, not sequential renames of the same work.

### 3. Triggers (unchanged from ADR-0085 §3 — restated for ownership)

K5-B **code** may start only when **≥1** holds **and** §6 successors are
accepted:

| Trigger | Signal |
| --- | --- |
| **Slot wall** | Product composition needs concurrent agents that honest `MAX_TASKS` + K5-S (+ optional paid K5-H) still cannot host |
| **Shape debt** | Measured cost: preempt/session/park complexity dominated by the pair (not taste) |
| **Host-class H3** | North-star layout requires session-schedulable agents ([ADR-0069](0069-harbor-host-class-north-star.md)) |

**Not** triggers: closing issue #25, finishing a horizon bar for its own sake,
demo census Full, desire for a process-like model without product need.

### 4. Relationship to K5-H

| Question | Answer in this design |
| --- | --- |
| Must K5-H land before K5-B? | **No.** If the product problem is “too many driver stacks/slots for N agents that each need a private session,” K5-H (multiplex) is the smaller step. If the product problem is “the pair shape itself blocks H3 / preempt complexity,” K5-B is the step. |
| Can both exist? | A world after K5-B may still multiplex **kernel** work; it does not need “M drivers for N agents” as the density story. Prefer **one** residual path for a given product pressure — document which in the code ADR. |
| Default recommendation under slot wall only | Prefer a **K5-H design ADR** first (ADR-0085 §5 checklist) — lower blast radius than pair collapse. |

This ADR does **not** open K5-H. A future K5-H design must still answer
ADR-0085 §5 before code.

### 5. ADRs that must be re-decided (or explicitly amended) under K5-B

A K5-B **code** series is invalid if it leaves these unspoken:

| ADR | Question K5-B must answer |
| --- | --- |
| **0006 / 0064 / 0068** | What is preempted — session quantum vs driver frame? Where does the trap frame live if there is no long driver stack? |
| **0016 / 0017** | Session lifetime vs task identity; publish of `CURRENT_EL0` when the “current” *is* the session |
| **0018** | Creator kill / fault: which identity ends; restart without a driver loop object |
| **0021 / 0022** | Manifest load creates sessions not driver `fn`s; park blocks the session, mask still one step |
| **0023** | **Superseded** (or successor) when code lands — this design does not supersede it yet |
| **0025 / 0031 / 0033 / 0038** | Reap/cascade/timeout refer to agent/session ids, not orphan driver loops |
| **0080 / 0088** | `home_cpu` attaches to the session/agent row (already composition data) |

No silent “we kept 0023 wording but deleted the driver.”

### 6. Code-series sketch (not authorized here)

Only after §3 trigger. Suggested split (each with its own ADR + gates):

| Slice | Intent | Evidence sketch |
| --- | --- | --- |
| **B0** | Inventory + pure model: session id, cost meters, no sched change | host tests |
| **B1** | Exception return → sched for **one** lab agent path (oracle feature) | QEMU line; product image unchanged |
| **B2** | Manifest loader creates sessions without per-agent driver `fn` | product-boot composition |
| **B3** | Preempt + park + creator policy on session identity | boot-check + SECURITY residual refresh |
| **B4** | Remove pair from architecture SSOT; supersede 0023 | docs gates |

Ordering may change in successor ADRs; **B0 before B1** is mandatory (pure
model first — same discipline as density/K7).

### 7. Standing product rules (carry forward)

| Rule | Meaning |
| --- | --- |
| No density via `MAX_TASKS++` alone | ADR-0085 / `oracle-census` |
| Prefer product evidence over oracle fleet growth | ADR-0085 §6 |
| Composition density | Prefer Thin/Mini / home pins; not Full for shallow agents |
| K5-B is not “Linux processes” | Still no ambient authority, still slot caps, still messages |

### 8. Non-goals (this ADR)

- Any code change to `sched` / `agent` / vectors  
- Superseding ADR-0023  
- K5-H implementation  
- Claiming H2 density complete  
- Forcing K5-B before a trigger  

## Consequences

### Positive

- K5-B is a **named design** with triggers, not an undefined residual.  
- Separates cleanly from K5-H and from paid K5-S.  
- Gives issue #25 a closeable design outcome without fake code progress.  
- Forces any future rewrite to list the ADR surface it reopens.

### Negative / costs

- Slot wall remains until K5-H or K5-B **code**.  
- Completeness track K5 stays “thin+Mini paid; H/B deferred” — honest.  
- Temptation to implement B1 “for fun” without trigger — **forbidden** by §3.

### Evidence for *this* design ADR

| Gate | Evidence |
| --- | --- |
| Docs | This file accepted; roadmap/architecture residual language cites K5-B → 0089 |
| Code | **None** (by design) |
| Issue #25 | Close as “design paid”; reopen or open B0 only when a trigger is recorded |

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| **Implement collapse now** | No §3 trigger; reopens four foundational ADRs; ROI fails after Mini + home_cpu |
| **Only K5-H design, skip K5-B doc** | Leaves “pair collapse” undefined; multi-role still names it as density frontier |
| **Supersede 0023 in this ADR** | Design without code must not rewrite the shape of record |
| **Equate K5-B with K5-H** | Multiplex ≠ collapse; different blast radii (ADR-0085 §1) |
| **Defer documenting forever** | Violates completeness honesty for a named residual |

## Related

- Shape of record: [0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md)  
- Residual policy: [0085](0085-k5-density-residual-design.md)  
- Stacks paid: [0044](0044-k5-agent-density.md), [0086](0086-k5-mini-stack-first-slice.md)  
- Product home: [0088](0088-product-home-cpu.md)  
- Completeness: [0026](0026-kernel-and-product-completeness.md)  
- Multi-role: [../reviews/2026-08-10-post-k8-multi-role.md](../reviews/2026-08-10-post-k8-multi-role.md) F-R7-2  
