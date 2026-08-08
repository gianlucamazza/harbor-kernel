# Contributing to Harbor

## Principles

**Before proposing work**, read the [mission and objectives](README.md#mission)
in the root README and [`docs/stack.md`](docs/stack.md) — what the project is
built with, and what is deliberately absent. Unfamiliar vocabulary:
[`docs/glossary.md`](docs/glossary.md). Work that contradicts either is a
successor ADR, not a patch.

1. **Boundary before code.** Structural changes need an ADR first
   ([ADR-0001](docs/adr/0001-multi-role-analysis.md)). Accepted ADRs are
   immutable; change them only via a successor — except narrow reconciliation
   amendments with an `amended:` frontmatter bump
   ([ADR-0058](docs/adr/0058-adr-amendments-and-mutation-freshness.md)).
2. **Evidence before “done”.** Host test, QEMU gate, or Pi stamp — see
   [`docs/verification.md`](docs/verification.md). Status vocabulary lives in
   [`docs/README.md`](docs/README.md).
3. **One owner per fact.** Completeness track status →
   [`docs/roadmap.md`](docs/roadmap.md) only. Do not copy full K/P tables into
   README or vision. After a status flip, refresh residual prose in
   `SECURITY.md` / `verification.md` / issue #17 only if they restate status.
4. **`make check` predicts CI.** Local green must mean remote green.
   Doc drift is a failed gate (`doc-claims`, `xrefs`, `doc-symbols`).

## Layout (scalable map)

```
docs/
  README.md          documentation map + ownership
  glossary.md        the words, and who owns each one
  stack.md           language, target, toolchain, features
  roadmap.md         K/P completeness SSOT
  architecture.md    model and layering, as it is today
  vision.md          mission sentence, product shape / horizons
  foundation-history.md  M0–M8 record (closed, not planning)
  adr/               immutable decisions
  design/            design contracts (not “done” claims)
  reviews/           dated findings
  verification.md    evidence index
scripts/
  check/             invariant gates
  boot/              product image + QEMU oracles
  agent/             agent-store pack/inject/inspect
  host/              SD, serial, blobs, mutants
  lib/               shared shell
crates/kernel-core/  pure, host-tested logic
src/                 kernel (arch, bsp, drivers, policy)
```

Details: [`docs/README.md`](docs/README.md), [`scripts/README.md`](scripts/README.md).

## Adding work

| Kind of change | Steps |
| --- | --- |
| Completeness track | Row in `docs/roadmap.md` → design ADR → code + gate → status flip |
| Stack assumption (toolchain, target, feature, host tool) | `docs/stack.md` in the same commit; a boundary move needs its ADR first |
| New term a reader will guess wrong | Row in `docs/glossary.md` pointing at the owning document |
| Gate | Script under `scripts/check/` or `scripts/boot/` → `Makefile` → README `make check` line stays in sync (`doc-claims`) |
| Agent composition | `scripts/agent/pack-store.py` + inject in product image path |
| Port (ISA/board) | [`docs/porting.md`](docs/porting.md), [`docs/arch-contract.md`](docs/arch-contract.md) |

## Before you claim complete

```bash
make check
```

Keep silicon transcripts and long evidence in `docs/verification.md`, not in
the public README.
