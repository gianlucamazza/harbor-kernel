# Harbor documentation

Map of the docs. The [root README](../README.md) is the public story; this page
says **which document owns which fact**.

## Start here

1. [**README**](../README.md) — what Harbor is, status, quick start  
2. [**Architecture**](architecture.md) — model, layering, foundation history, [completeness roadmap](architecture.md#completeness-roadmap)  
3. [**Vision**](vision.md) — product OS shape and use cases  

Then, as needed: [`SECURITY.md`](../SECURITY.md) (authority / threats) ·
[`verification.md`](verification.md) (evidence).

## By goal

| Goal | Start | Then |
| --- | --- | --- |
| Build and boot | [README](../README.md) | [boot-chain](boot-chain.md), [hardware](hardware.md) |
| Understand the agent model | [architecture](architecture.md) | [differs §](architecture.md#how-harbor-differs-from-a-traditional-kernel), [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) |
| Completeness roadmap (K/P) | [architecture § completeness](architecture.md#completeness-roadmap) | [ADR-0026](adr/0026-kernel-and-product-completeness.md) |
| Product vision / use cases | [vision](vision.md) | architecture, SECURITY |
| Authority and threats | [SECURITY](../SECURITY.md) | architecture agent model |
| Verify a claim | [verification](verification.md) | linked transcript or gate |
| Port ISA/board | [porting](porting.md) | [arch-contract](arch-contract.md) |
| Structural decision | [adr/](adr/README.md) | the linked ADR |

## Ownership and status vocabulary

| Document | Owns | Does not own |
| --- | --- | --- |
| `README.md` | Public story, status snapshot, quick start | Full evidence transcripts |
| `docs/architecture.md` | Normative model, layering, foundation + K/P roadmap | Product narrative |
| `docs/vision.md` | Product shape, horizons, use cases | Silicon status claims |
| [ADR-0026](adr/0026-kernel-and-product-completeness.md) | Completeness as goal | Per-track design |
| `SECURITY.md` | Threat model, authority surface, residuals | Roadmap ordering |
| `docs/verification.md` | Gates, transcripts, blind spots | Normative design |
| `docs/adr/*.md` | Immutable structural decisions | Live dashboard |
| `docs/reviews/*.md` | Dated findings | Current truth after later fixes |
| `docs/design/*.md` | Design contracts | Completion claims without evidence |

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
  irqtable, layout, manifest, paging, poll, prog, reset, ring, rng,
  runqueue, rxline, spi, syscall, tasks, textgrid, timer, uart, wake
  tests/ public_api, model_sched, model_ipc
src/
  arch/           ISA facade and AArch64 entry, exceptions, MMU, switch, EL0
  bsp/            board selection, Raspberry Pi memory map, GPIO and bindings
  drivers/        PL011, GICv2, RNG200, power management and optional display
  irq/            IRQ ownership, masking, counters and dispatch wiring
  bootstrap/      boot sequence, loader, console server, demos and self-tests
  agent/          EL0 agent shell and session lifecycle
  sched/          TCBs, stacks, context switching and wake queue
  ipc/            kernel IPC policy and capability translation
  mm/             heap, address spaces, frames, layout and task stacks
  console.rs      kernel TX/RX policy
  main.rs         bootstrap entry
  panic.rs        panic and halt path
  status.rs       optional display status slots
  sync.rs         shared-state cell
  time.rs         tick policy
boot/             Raspberry Pi firmware configuration
scripts/          build, layering, documentation and QEMU gates
```

## Decision records and reviews

[`adr/README.md`](adr/README.md) — ADR lifecycle. Accepted ADRs are immutable;
change requires a successor. [`reviews/`](reviews/) — dated findings, not live
status. Active work: [completeness roadmap](architecture.md#completeness-roadmap);
foundation history: [architecture § roadmap](architecture.md#roadmap).

## Documentation checks

```bash
make doc-claims
make doc-symbols
make xrefs
```

These run inside `make check`. They compare sets, links, and paths — not whether
prose is conceptually honest.
