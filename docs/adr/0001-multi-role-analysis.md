---
id: 0001
title: Multi-role analysis as project gate before M3
status: accepted
date: 2026-08-04
accepted: 2026-08-04
---

# ADR-0001: Multi-role analysis as project gate before M3

## Acceptance

**Accepted 2026-08-04.** The baseline multi-role pass ran the same day
([report](../reviews/2026-08-04-multi-role.md)); findings have driven ADRs and
code (F12→ADR-0006, F24→`make layering`, free-list integrity, exception stack,
doc-claims). The process is live, not aspirational. Immutable under the ADR
lifecycle: change only via a successor ADR.

## Context

`rpi_minimal_agentic` has passed the bring-up milestones up to P2 (early MMU,
softfloat, build-enforced gates, free-list heap, W^X, WFI idle) and is ready, as
far as declared dependencies go, for M3 (cooperative tasks).

The failure surface of a bare-metal kernel is asymmetric:

- QEMU/TCG does not reproduce the behaviour of exclusives on Device-nGnRnE; a
  green `make boot-check` is not evidence about memory attributes, caches or
  firmware state (see [`verification.md`](../verification.md)).
- Protections (W^X, guard page) hold only if someone has seen them fire on
  hardware; a map that "activates" does not demonstrate enforcement.
- When this ADR was written, the layering rules in
  [`architecture.md`](../architecture.md) were explicit but not enforced by
  tooling — discipline plus human review only. That gap was finding F24; it is
  **closed for import edges** by `make layering` (`scripts/check-layering.sh`).
  What remains review-only is coupling that is not an import (shared constants,
  agreed register values, naming conventions) — the gate's documented blind
  spot, not a claim that layering is ungated.
- Before introducing an execution abstraction (task / yield / scheduler),
  unexamined choices risk solidifying underneath M3.

The automated gates remain necessary and insufficient. What is needed is a
repeatable, multi-perspective inventory producing actions or explicit _accepted
risk_ — not a one-off monolithic code review.

## Decision

Adopt a **fixed-role multi-role review** as a project discipline.

### Cadence

1. **Full baseline** before M3 (first pass: report in
   [`docs/reviews/`](../reviews/)).
2. **Incremental re-run** on diffs touching memory, IRQ/`unsafe`, the boot chain
   or layering boundaries, before marking a milestone `done (HW)`.
3. Findings of the _architectural boundary_ kind (boundaries, security model,
   ABI) → a **dedicated ADR** before the code that implements them.

### Fixed roles

| ID  | Role                            | Focus                                        |
| --- | ------------------------------- | -------------------------------------------- |
| R1  | Layering architect              | arch/`bsp`/`drivers`/`irq`/`exception` rules |
| R2  | Memory / MMU                    | Early map, W^X, layout, heap, tables         |
| R3  | Interrupts / concurrency / idle | GIC, timer, ring, atomics, WFI               |
| R4  | `unsafe` and panic audit        | Inventory, invariants, halt path             |
| R5  | Verification and blind spots    | Gates, CI, what a green does not prove       |
| R6  | Boot chain and firmware         | EL2→EL1, blobs, DTB, deploy                  |
| R7  | Performance and footprint       | Size, latency, idle, alloc (measured)        |
| R8  | Tooling / CI / DX               | Makefile, scripts, toolchain, onboarding     |
| R9  | Pre-agent security              | EL1 surface, MMIO, capability prerequisites  |
| R10 | Agent roadmap (M3–M6)           | Readiness gaps, not design fantasy           |
| R11 | Documentation                   | docs↔code drift, honest claims               |
| R12 | `kernel-core` API               | Pure logic, testability, boundaries          |

### Finding taxonomy

Per role: **problems**, **improvements**, **optimisations** (the last only with a
metric or a falsifiable hypothesis).

Severity:

| Tag             | Meaning                                      |
| --------------- | -------------------------------------------- |
| `P0`            | Correctness, hardware hang, safety           |
| `P1`            | Debt that blocks M3+ or regresses a gate     |
| `P2`            | Quality / DX                                 |
| `P3`            | Nice-to-have                                 |
| `Risk-accepted` | Seen, deliberately not fixed, with rationale |

### Artefacts

| What                         | Where                                         |
| ---------------------------- | --------------------------------------------- |
| This decision                | `docs/adr/0001-…` (immutable once `accepted`) |
| ADR index                    | [`README.md`](README.md)                      |
| Outcome of a pass            | `docs/reviews/YYYY-MM-DD-multi-role.md`       |
| Derived structural decisions | ADRs `0002+`                                  |

The process ADR does **not** list bugs. Findings live in the report; if a finding
changes a boundary or a model, it becomes a subsequent ADR.

### Finding format (in the report)

```markdown
### [P1] R2 — short title

- **Aspect:** problem | improvement | optimisation
- **Evidence:** path:line, or a doc quotation / observed behaviour
- **Impact:** …
- **Proposed action:** fix | ADR | test | risk-accepted
- **Effort:** S | M | L
```

## Consequences

**Positive**

- Traceability: every finding has a role, evidence, severity and next step.
- Aligns human review with the already-documented blind spots of the automated
  gates.
- Institutionalises a pre-milestone checkpoint (in particular pre-M3).
- Separates process (this ADR) from outcome (the report) and from individual
  decisions (successor ADRs).

**Negative / costs**

- Time cost (order of 1–2 sessions for a full pass); mitigated by per-role
  checklists and by re-running only on relevant diffs.
- Risk of _review theatre_: many findings, no backlog. Mitigation: every `P0`/`P1`
  requires an action or an explicit `Risk-accepted`; a role may close with "no
  material findings" plus rationale.

## Alternatives considered

| Alternative                         | Why not chosen                                                                                          |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Automated gates / CI only           | Still blind to attributes, caches, firmware, non-import coupling, and roadmap — gates do not replace R1–R12 |
| Monolithic single-role code review  | Loses perspectives (security vs size vs IRQ latency)                                                    |
| A one-off external audit            | Does not institutionalise the pre-milestone discipline                                                  |
| Formal methods / model checking now | Low ROI pre-M3; host tests plus Miri on `kernel-core` suffice as a foundation                           |

## References

- [`docs/architecture.md`](../architecture.md) — layering and milestones
- [`docs/verification.md`](../verification.md) — gates and blind spots
- First pass:
  [`docs/reviews/2026-08-04-multi-role.md`](../reviews/2026-08-04-multi-role.md)
