# Architecture Decision Records

Structural decisions. Lifecycle:

1. An ADR starts `proposed`, with Context / Decision / Consequences /
   Alternatives.
2. A human accepts it, possibly with refinements → `accepted`.
3. An `accepted` ADR is **immutable**. To change it, write a successor and mark
   the old one `superseded` (linked through `related`).

Numbering is monotonic; never renumber. Prefer a separate `docs:` commit from
the code that follows.

| ID                                      | Title                                                | Status   |
| --------------------------------------- | ---------------------------------------------------- | -------- |
| [0001](0001-multi-role-analysis.md)     | Multi-role analysis as project gate before M3        | accepted |
| [0002](0002-softfloat-kernel.md)        | Kernel compiled softfloat, FP left trapping          | accepted |
| [0003](0003-early-mmu.md)               | MMU enabled before any Rust runs                     | accepted |
| [0004](0004-gic-group0-firmware-pin.md) | GIC Group 0 with IAR/EOIR, and the firmware pin      | accepted |
| [0005](0005-static-page-table-arena.md) | Static page-table arena instead of a frame allocator | accepted |
| [0006](0006-cooperative-execution-model.md) | Cooperative execution model (M3 tasks)           | proposed |

Operational reviews (findings, not decisions): [`../reviews/`](../reviews/).

## Why 0002–0006 exist

0001 institutionalised _how_ to review before anything recorded _what_ had been
decided: someone arriving and asking "why softfloat?" had no answer. 0002–0005
cover the four choices that already constrain the running kernel and are
**accepted** with the evidence each ADR names (gates seen red, or silicon, or
both). 0006 records the execution model **before** any scheduler code (finding
F12): cooperative tasks, heap stacks with unmapped guards, no preemption, no
IRQ-side switch. It stays **`proposed`** until a human accepts the model or M3
implements it — see the Acceptance status section in that ADR.

Each names **the gate that would catch its own reversal**, and for three of them
that gate has been seen red — see the mutation table in
[`../verification.md`](../verification.md). 0005 declares a weak console-only
remainder signal; 0006 declares process gates until M3 tests exist. That honesty
is part of the record.
