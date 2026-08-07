# Contributing to Harbor

## Principles

1. **Boundary before code.** Structural changes need an ADR first
   ([ADR-0001](docs/adr/0001-multi-role-analysis.md)). Accepted ADRs are
   immutable; change them only via a successor.
2. **Evidence before “done”.** Host test, QEMU gate, or Pi stamp — see
   [`docs/verification.md`](docs/verification.md). Status vocabulary lives in
   [`docs/README.md`](docs/README.md).
3. **One owner per fact.** Completeness track status →
   [`docs/roadmap.md`](docs/roadmap.md) only. Do not copy full K/P tables into
   README or vision.
4. **`make check` predicts CI.** Local green must mean remote green.

## Layout (scalable map)

```
docs/
  README.md          documentation map + ownership
  roadmap.md         K/P completeness SSOT
  architecture.md    model, layering, foundation history
  vision.md          product shape / horizons
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
| Gate | Script under `scripts/check/` or `scripts/boot/` → `Makefile` → README `make check` line stays in sync (`doc-claims`) |
| Agent composition | `scripts/agent/pack-store.py` + inject in product image path |
| Port (ISA/board) | [`docs/porting.md`](docs/porting.md), [`docs/arch-contract.md`](docs/arch-contract.md) |

## Before you claim complete

```bash
make check
```

Keep silicon transcripts and long evidence in `docs/verification.md`, not in
the public README.
