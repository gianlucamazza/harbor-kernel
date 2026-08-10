---
id: 0085
title: K5 density residual policy — stack, half, pair-collapse; first-slice choice
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0001, 0006, 0016, 0017, 0018, 0021, 0022, 0023, 0026, 0044, 0084]
---

# ADR-0085: K5 residual policy after thin stacks (design)

## Acceptance status

**Accepted as design / policy** (2026-08-10). The K5 **first mechanism**
slice (thin stacks) is already **done (HW)**
([ADR-0044](0044-k5-agent-density.md)). What remained was an undifferentiated
residual (“driver-half collapse”) that mixed a small stack refinement, a
medium multiplex redesign, and a large pair rewrite. This ADR **splits and
governs** those residuals and named the **first code slice**
([ADR-0086](0086-k5-mini-stack-first-slice.md) — **done (HW)** 2026-08-10).

This document does **not** implement shared drivers or session-as-schedulable
(**K5-H/B** remain deferred under §1–§3).

## Context

| Piece | Status |
| --- | --- |
| Agent = EL1 driver task + EL0 program | **Shape of record** — [ADR-0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md) |
| Schedulable entity | Driver task only; session in TCB; pair not collapsed |
| Stack classes | `Full` 16 KiB / `Thin` 4 KiB (+ guard) — [ADR-0044](0044-k5-agent-density.md) **done (HW)** |
| `MAX_TASKS` | oracle census tax (see architecture table / `oracle-census`; not a density design) |
| Cost axes | **Slot** (one task per agent) · **heap stack** · **AS frames** (option C clone — [ADR-0084](0084-k7-residual-policy.md) K7-T if *map* density) |
| Preemption / SMP / EL0-on-CPU1 / steal | **done (HW)**; agents non-stealeable without TLB IPI |
| Roadmap next (at acceptance) | “K5 driver-half” without residual IDs — superseded in prose by K5-S/H/B after this ADR; **K5-S** later **done (HW)** [ADR-0086](0086-k5-mini-stack-first-slice.md) |

Thin stacks fix **heap per worker**. They do **not** remove one slot per
agent, nor the synchronous driver loop cost named in ADR-0023. Raising
`MAX_TASKS` for more oracle demos is the opposite of a density win.

## Decision

### 1. Split the residual (mandatory project language)

After this ADR, status prose **must not** say only “K5 driver-half residual”
as a single blob. Use:

| Residual ID | Kind | Status vocabulary |
| --- | --- | --- |
| **K5-S** Stack refinement | Smaller / more stack classes; product defaults | First **code** under this policy (~0086) |
| **K5-H** Driver half / multiplex | M driver tasks serving N agent sessions (N≫M) concurrent | `deferred` until K5-S paid **and** slot pressure is product-real; then **dedicated design+code** ADRs |
| **K5-B** Pair collapse | EL0/session as schedulable entity (no long-lived driver frame) | `deferred` with triggers (§3); multi-ADR rewrite of 0006/0016/0017/0018/0023 |

K5 thin (0044) remains **done (HW)**. Completing K5-H or K5-B is **not**
required to claim that first slice closed.

### 2. What “density” means (three meters)

| Meter | Bottleneck | Owned by |
| --- | --- | --- |
| **Slots** | Concurrent agents ≤ `MAX_TASKS` − idle − infrastructure | K5-H (later); never by silent `MAX_TASKS++` alone |
| **Heap stacks** | Guard + usable per task | K5-S (now) + Thin/Full (0044) |
| **AS / frames** | Clone kernel maps per agent AS | K7-T if trigger (0084); **orthogonal** to K5-S/H |

A proposal that only raises `MAX_TASKS` **fails** this ADR’s density bar.

### 3. Triggers for K5-B (pair collapse) — only

K5-B may leave `deferred` only when **at least one** holds and a design ADR
is accepted:

| Trigger | Signal |
| --- | --- |
| **Slot wall** | Product composition needs concurrent agents that K5-S + honest `MAX_TASKS` and a paid K5-H still cannot host |
| **Shape debt** | Preempt/session/park complexity dominated by the pair (measured, not aesthetic) |
| **Host-class H3** | North-star layout requires session-schedulable agents ([ADR-0069](0069-harbor-host-class-north-star.md)) |

**Not** triggers: “finish the roadmap row”, demo census Full, desire for
Linux-like process model.

### 4. First code slice (~0086) — **K5-S only**

| Decision | Choice |
| --- | --- |
| Scope | Add stack class **`Mini`** (one **4 KiB** page, **no** unmapped guard — see code ADR-0086; 2 KiB+guard is impossible on a 4 KiB granule); pure density accounting; product/loader **may** prefer Thin/Mini for shallow agents; oracle reports class costs |
| Non-scope | Shared driver pool (K5-H); session-as-schedulable (K5-B); TTBR1; agent steal; dynamic stack growth |
| Default `spawn*` | Remain **Full** (no silent shrink of deep drivers / demos) |
| New API | `spawn_mini` / class parameter; loader policy table for agent bodies |
| Pure model | Extend `kernel_core::density`: `Mini`, `bytes_per_task`, optional `AgentCost` breakdown (slot flag + stack class) — host-tested |
| Oracle | Greppable `density: mini n=… bytes_each=…` (and keep thin line); no claim of unlimited agents |
| `MAX_TASKS` | **Must not** rise solely to make the oracle pass Mini; census of oracle tasks documented in verification or bootstrap comment |
| Evidence | host tests → `make boot-check` → Pi stamp → verification row |

**Why K5-S first (not K5-H):** same discipline as steal-then-agent-TLB and
option-C-then-TTBR1 — pay the **small, reversible** meter (heap) with clear
gates before re-opening session ownership, park identity, and creator/fault
referents (K5-H/B). ADR-0023 already forbids sneaking pair collapse in as a
side effect.

**Why Mini = no-guard single page (not 2 KiB + hole):** the page is the
unmap granule; a half-page guard cannot be unmapped. Mini therefore drops the
guard page and keeps one full usable page (half of Thin’s heap). First code
restricts Mini to short EL1 workers (`spawn_mini`); multi-SVC agent drivers
stay Full/Thin.

### 5. K5-H constraints (design only — not first code)

If/when a K5-H design ADR opens, it **must** answer before code:

1. **Identity:** is “the task” in ADR-0018 the driver or the session?  
2. **CURRENT_EL0 / publish:** one driver, many sessions — publish on switch-in of session, never cross-session.  
3. **Park/recv (0022):** park parks the driver; other sessions on that driver must not run.  
4. **Stealeable / home:** driver home vs session affinity.  
5. **Oracle:** N sessions on M drivers with visible progress without TX from CPU1 if pinned.  

Until that ADR exists, implementations that multiplex sessions on one task
are **out of model**.

### 6. Product and excellence (standing rules)

| Rule | Meaning |
| --- | --- |
| **No density via `MAX_TASKS++` alone** | Raises need a census rationale (oracle feature tax vs product) |
| **Oracle vs product** | Prefer strengthening product-boot-check over growing oracle fleets ([multi-role 2026-08-10](../reviews/2026-08-10-post-k8-multi-role.md) F-R5-2 / F-R7-1) |
| **Composition density** | Manifest agents should not require Full unless the body needs deep EL1 |
| **Orthogonality** | K5-S does not claim AS frame wins; K7-T does not claim stack wins |

### 7. Non-goals (this policy + first code)

- Pair collapse (K5-B) implementation  
- Shared driver multiplex (K5-H) implementation  
- Dynamic stack growth / guardless stacks  
- Coupling K5 to TTBR1 or agent+TLB steal  
- Claiming “unlimited agents” or finishing H2 solely by Mini  
- Silent default Full → Mini for all existing demos  

## Consequences

### Positive

- Residual language matches 0084-style honesty (split meters, trigger gates).  
- First code is small, host-testable, and does not re-decide the pair.  
- ADR-0023’s cost baseline remains the argument surface for K5-H/B.  
- Stops treating `MAX_TASKS` ratchet as K5 progress.

### Negative / costs

- First code **does not** remove one-slot-per-agent; slot wall remains until K5-H/B.  
- Mini risks stack overflow if mis-applied to deep drivers — code must scope use.  
- Two more ADRs before true “half collapse” (H then maybe B).

### Evidence expectations (first code ~0086)

| Gate | Evidence |
| --- | --- |
| Host | `density` Mini arithmetic (+ tests) |
| QEMU | `density: mini n=` (and thin still present) |
| HW | same lines + `hw-transcript-check`; Cortex-A72 stamp |
| Docs | roadmap K5 row uses K5-S/H/B; verification row for 0086 |

## Alternatives considered

| Alternative | Why not for *first* code |
| --- | --- |
| **K5-H multiplex first** | Touches park, publish, creator policy; needs its own design depth; higher risk after K8 just closed |
| **K5-B pair collapse first** | ADR-0023: different kernel; reopens four ADRs; wrong ROI before Mini/Thin product defaults |
| **Only census / Thin-default policy (no Mini)** | Good excellence work but does not extend the stack meter; can ship *with* 0086, not instead of a mechanism slice if we claim K5-S progress |
| **Raise MAX_TASKS** | Explicitly rejected as density solution (0044 §4 restated) |
| **Defer all K5 residual forever** | Violates ADR-0026 completeness for a named H2 residual without exclusion ADR |

## Related

- Shape: [0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md)  
- Thin: [0044](0044-k5-agent-density.md)  
- Completeness: [0026](0026-kernel-and-product-completeness.md)  
- Map density orthog: [0084](0084-k7-residual-policy.md)  
- Multi-role inventory: [../reviews/2026-08-10-post-k8-multi-role.md](../reviews/2026-08-10-post-k8-multi-role.md)  
- Code follow-on: [0086](0086-k5-mini-stack-first-slice.md) (K5-S Mini first slice)
- K5-B design: [0089](0089-k5-b-pair-collapse-design.md) (no code until trigger)
