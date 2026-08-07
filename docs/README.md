# Harbor documentation

Map of the docs. The [root README](../README.md) is the public story; this page
says **which document owns which fact**.

## Start here

1. [**README**](../README.md) — what Harbor is, status, quick start  
2. [**Roadmap**](roadmap.md) — mission, H0–H2 product path, K/P status SSOT  
3. [**Architecture**](architecture.md) — model, layering, foundation history  
4. [**Vision**](vision.md) — product OS shape and use cases  

Then, as needed: [`SECURITY.md`](../SECURITY.md) · [`verification.md`](verification.md) ·
[`CONTRIBUTING.md`](../CONTRIBUTING.md).

## By goal

| Goal | Start | Then |
| --- | --- | --- |
| Build and boot | [README](../README.md) | [boot-chain](boot-chain.md), [hardware](hardware.md) |
| Completeness + product path (K/P) | [roadmap](roadmap.md) | [ADR-0026](adr/0026-kernel-and-product-completeness.md), [vision](vision.md) |
| Understand the agent model | [architecture](architecture.md) | [differs §](architecture.md#how-harbor-differs-from-a-traditional-kernel), [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) |
| Product vision / use cases | [vision](vision.md) | architecture, SECURITY |
| Authority and threats | [SECURITY](../SECURITY.md) | architecture agent model |
| Verify a claim | [verification](verification.md) | linked transcript or gate |
| Port ISA/board | [porting](porting.md) | [arch-contract](arch-contract.md) |
| Structural decision | [adr/](adr/README.md) | the linked ADR |
| Extend the tree | [CONTRIBUTING](../CONTRIBUTING.md) | [scripts map](../scripts/README.md) |

## Ownership and status vocabulary

| Document | Owns | Does not own |
| --- | --- | --- |
| `README.md` | Public story, status snapshot, quick start | Full K/P tables, evidence transcripts |
| [`roadmap.md`](roadmap.md) | **Mission, H0–H2 outcomes, H1 order, K/P status** (SSOT) | Per-track design ADRs |
| `docs/architecture.md` | Normative model, layering, foundation history | Live K/P table copies |
| `docs/vision.md` | Product shape, horizons, use cases | Silicon status claims |
| [ADR-0026](adr/0026-kernel-and-product-completeness.md) | Completeness as goal | Per-track design |
| `SECURITY.md` | Threat model, authority surface, residuals | Roadmap ordering |
| `docs/verification.md` | Gates, transcripts, blind spots | Normative design |
| `docs/adr/*.md` | Immutable structural decisions | Live dashboard |
| `docs/reviews/*.md` | Dated findings | Current truth after later fixes |
| `docs/design/*.md` | Design contracts | Completion claims without evidence |
| `scripts/README.md` | Script taxonomy | Kernel architecture |

Status labels:

- **implemented** — in the tree now  
- **done (QEMU)** / **done (HW)** — exercised under that evidence  
- **open** — on the completeness roadmap or awaiting a decision  
- **proposed** — design exists, not accepted/complete  
- **historical** — true at a past date  

Update the **owning** document when a fact changes; do not duplicate full status
tables for convenience.

## Code layout

The following map is checked against the source tree. It describes ownership,
not every symbol.

```
crates/kernel-core/  pure logic, host-tested — no MMIO, no assembly:
  a64, agentstore, bump, cap, delay, display, font8x8, frame, gic, heap, ipc,
  irqcap, irqtable, irqwait, layout, manifest, paging, poll, prog, reset, ring, rng,
  runqueue, rxline, spi, syscall, tasks, textgrid, timer, uart, wake
  tests/ public_api, model_sched, model_ipc
src/
  arch/           ISA facade and AArch64 entry, exceptions, MMU, switch, EL0
  bsp/            board selection, Raspberry Pi memory map, GPIO and bindings
  drivers/        PL011, GICv2, RNG200, power management and optional display
  irq/            IRQ ownership, masking, counters, wait port, notification caps
  bootstrap/      boot sequence, loader, console server, demos and self-tests
  agent/          EL0 agent shell and session lifecycle
  sched/          TCBs, stacks, context switching and wake drain
  ipc/            kernel IPC policy and capability translation
  mm/             heap, address spaces, frames, layout and task stacks
  console.rs      kernel TX/RX policy
  main.rs         bootstrap entry
  panic.rs        panic and halt path
  status.rs       optional display status slots
  sync.rs         shared-state cell
  time.rs         tick policy
boot/             Raspberry Pi firmware configuration
scripts/          check/ boot/ agent/ host/ lib/ — see scripts/README.md
```

## Current truth (consolidation pointer)

Do **not** treat this block as a second status table — it only steers readers.
**Status lives in [roadmap.md](roadmap.md).**

| Layer | State (2026-08-07) |
| --- | --- |
| **H0 foundation** | **done (HW)** on Pi 4B (M0–M8 + parked cancel) |
| **H1 first slices** | **done (QEMU):** K1, K2 auto-reap, K3 revoke, K10 reap/restart, K6, P1, P6 |
| **H1 next** | K9 → P5 → P2 → (P3\|P4) · K5 |
| **Residuals** | K2 timeout · K3 transfer · K10 creator-exit cascade · H2 · HW stamps |
| **Standing watch** | [#14](https://github.com/gianlucamazza/harbor-kernel/issues/14) SpiDevice / ADR-0020 |

## Decision records and reviews

[`adr/README.md`](adr/README.md) — ADR lifecycle. Accepted ADRs are immutable;
change requires a successor. [`reviews/`](reviews/) — dated findings, not live
status. Active work: [roadmap](roadmap.md); foundation history:
[architecture § roadmap](architecture.md#roadmap).

## Documentation checks

```bash
make doc-claims
make doc-symbols
make xrefs
```

These run inside `make check`. They compare sets, links, and paths — not whether
prose is conceptually honest.
