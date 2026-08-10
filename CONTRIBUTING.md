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
   [`docs/README.md`](docs/README.md). A status flip needs a row in the evidence
   index (`make roadmap-evidence`).
3. **One owner per fact.** Completeness track status →
   [`docs/roadmap.md`](docs/roadmap.md) only. Do not copy full K/P tables into
   README or vision. After a status flip, refresh residual prose in
   `SECURITY.md` / `verification.md` only if they restate status — and **close
   or update any GitHub issue that tracked the paid work** (do not leave a second
   stale tracker; #14 is a standing watch, not a status table).
4. **`make check` predicts CI.** Local green must mean remote green.
   Doc drift is a failed gate (`doc-claims`, `xrefs`, `doc-symbols`).
5. **Ship path ≠ lab path.** The oracle image (`make boot-check`) proves
   subsystem demos. The **product** image is proven by
   `make product-boot-check` (composition minimum on lines the shipped path
   already prints) and kept free of demo strings by `make product-builds`.
   Prefer strengthening the product gate over growing the oracle fleet
   ([ADR-0085](docs/adr/0085-k5-density-residual-design.md)).
6. **`MAX_TASKS` is not density.** Raising the ceiling for concurrent demos is
   oracle tax. Update `scripts/check/oracle-census.sh` (and the architecture
   capacity table) in the same commit; density wins are stack classes / K5, not
   `MAX_TASKS++`.

## Layout (scalable map)

**Where new code goes:** [`docs/design/project-topology.md`](docs/design/project-topology.md)
(ISA / board / lab / pure axes). Do not invent a second package for a lab ISA.

```
docs/
  README.md          documentation map + ownership
  design/            topology, multi-arch, progressive ISA (contracts)
  adr/               immutable decisions
  roadmap.md         K/P completeness SSOT
  architecture.md    model and layering today
  verification.md    evidence index
scripts/
  check/             invariant gates
  boot/              product + lab oracles
  agent/ host/ lib/
crates/kernel-core/  pure, host-tested logic (no MMIO)
src/
  main.rs            product vs lab dispatch only
  arch/ bsp/ drivers/
  lab/               lab maturity path (thin bring-up)
  bootstrap/ …       product policy
```

Details: [`docs/README.md`](docs/README.md), [`docs/design/README.md`](docs/design/README.md),
[`scripts/README.md`](scripts/README.md).

## Adding work

| Kind of change | Steps |
| --- | --- |
| Completeness track | Row in `docs/roadmap.md` → design ADR → code + gate → status flip → evidence row → close/update tracker issues |
| Stack assumption (toolchain, target, feature, host tool) | `docs/stack.md` in the same commit; a boundary move needs its ADR first |
| New term a reader will guess wrong | Row in `docs/glossary.md` pointing at the owning document |
| Gate | Script under `scripts/check/` or `scripts/boot/` → `Makefile` → README `make check` line stays in sync (`doc-claims`) |
| Product-path claim | Prefer a product-boot assert (or invariant beacon) over a new oracle demo; demos stay behind `feature = "oracle"` |
| Concurrent-demo slot need | Justify vs density (ADR-0085); bump `oracle-census` `EXPECTED_MAX_TASKS` + architecture table with the ADR reason |
| Agent composition | `scripts/agent/pack-store.py` + inject in product image path |
| Port (ISA/board) | [`docs/porting.md`](docs/porting.md), [`docs/arch-contract.md`](docs/arch-contract.md) |

## Before you claim complete

```bash
make check
```

Keep silicon transcripts and long evidence in `docs/verification.md`, not in
the public README.
