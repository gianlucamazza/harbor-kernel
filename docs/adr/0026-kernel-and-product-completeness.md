---
id: 0026
title: Completeness of the Harbor kernel and product OS is the project goal
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0001, 0006, 0007, 0021, 0023, 0024, 0025]
---

# ADR-0026: Completeness is the goal — kernel and product OS

## Acceptance status

**Accepted** (2026-08-07) by the project owner. This ADR records a **goal and
roadmap policy**. It does not implement preemption, networking, or any
completeness track.

## Context

Harbor’s foundation (M0–M8 and parked-task cancel) is stamped on silicon. The
public story still often reads as “lab kernel, intentionally incomplete,” with
preemption, SMP, ASID, IRQ-wait, storage, and network listed as permanent-looking
non-goals.

That framing was useful while bringing up boundaries. It is the wrong identity
for the next phase. The project’s objective is **completeness**: a finished
agent-based **microkernel** and a **product operating system** built on it —
not Linux/POSIX parity, and not “leave mechanisms unfinished forever.”

Without a recorded decision, every residual risks ossifying into a non-goal, and
the roadmap collapses to an empty frontier after M8.

## Decision

### 1. Project goal

Harbor aims to complete:

| Track | Meaning |
| --- | --- |
| **K — kernel** | Every core mechanism of the agent-based microkernel: execution, wait model, IPC/capabilities, isolation, load, lifecycle, multi-core when required — designed, gated, and demonstrated |
| **P — product OS** | Deliverable system services and platform paths (composition, storage, network, display/input, naming, tooling) under the same agent/capability model, preferably as agents rather than TCB growth |

“Complete” means **not permanently missing**, not “large.” Completeness is
compatible with a small TCB and a non-POSIX ABI.

### 2. Temporary gaps vs permanent exclusions

| Class | Rule |
| --- | --- |
| **Not yet complete** | Honest residual; listed on the completeness roadmap in [`architecture.md`](../architecture.md); needs a design ADR before a boundary moves ([ADR-0001](0001-multi-role-analysis.md)) |
| **Out of model** | Explicit permanent non-goals: Linux/POSIX compatibility; hiding platform firmware blobs; multi-tenant cloud hypervisor (unless a future ADR owns it) |

Former “non-goal until ADR” items (preemption, IRQ-wait, timeout/auto-reap,
ASID, SMP, USB host, full framebuffer, external load, …) are **completeness
tracks**, not permanent refusals. ADR-0006’s cooperative model remains in force
until a **successor ADR** replaces it — this ADR does not invent preemption.

### 3. Completeness never weakens evidence

A track is not done when code merges alone. The same discipline as M/P
milestones applies: host tests and gates where pure; QEMU and hardware stamps
where attributes, firmware, or devices matter ([`verification.md`](../verification.md)).

### 4. Roadmap ownership

The ordered **K** and **P** tracks live in
[`architecture.md` § Completeness roadmap](../roadmap.md).
Order may change without a new policy ADR; **dropping the goal of completeness**
or reclassifying a K/P item as a permanent non-goal without exclusion rationale
requires a successor to this ADR.

### 5. Relation to vision

[`vision.md`](../vision.md) describes product shape and use cases. This ADR
makes completeness the **committed goal**; vision remains free to narrate
horizons without claiming silicon status.

## Consequences

### Positive

- README and architecture can say **not yet complete** without celebrating
  incompleteness.
- Residuals in `SECURITY.md` map to tracks (open until K*) instead of “forever
  out of scope,” where that is accurate.
- Contributors know that post-M8 work is expected, not optional hobby.

### Negative / costs

- Expectation management: product OS tracks are large; the goal does not invent
  a ship date.
- Existing ADR text that says “non-goal” for timeout or preemption stays
  immutable; readers must learn “non-goal *of that ADR*” ≠ “project never.”
- Pressure to rush tracks without design ADRs — mitigated by Decision §2–3.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Stay a perpetual lab with empty frontier | Contradicts the stated product direction |
| Completeness = Linux parity | Betrays capability composition identity ([ADR-0007](0007-project-identity-harbor-kernel.md)) |
| Kernel-only completeness, no product OS | Rejected by project owner: product OS is in scope |
| Implement preemption inside this ADR | Policy only; preemption needs its own design ADR |

## Gates that catch reversal

| Reversal | Gate |
| --- | --- |
| Docs claim permanent non-goal for a K/P mechanism without exclusion | Review + `architecture.md` completeness section |
| Track marked done without evidence | Existing verification / boot-check culture; per-track ADRs name gates |
| This goal silently dropped | Successor ADR required; architecture artefact table lists 0026 |

## References

- [`../architecture.md`](../architecture.md) — completeness roadmap
- [`../vision.md`](../vision.md) — product shape
- [ADR-0001](0001-multi-role-analysis.md) — ADR before boundary moves
- [ADR-0006](0006-cooperative-execution-model.md) — cooperative until successor
- [ADR-0025](0025-cancel-blocked-wait.md) — cancel without timeout (timeout is K2)
