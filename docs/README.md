# Harbor documentation

Map of the docs. The [root README](../README.md) is the public story; this page
says **which document owns which fact**.

## The 5-minute path

For a first visit, in this order — you can stop at any step and still have a
correct picture:

| #   | Question                                     | Read                                                                                                                            | Minutes |
| --- | -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ------- |
| 1   | Mission, objectives, stack                   | [root README](../README.md)                                                                                                     | 2       |
| 2   | What is an _agent_ here (it is not an LLM)   | [glossary](glossary.md), then [architecture § How Harbor differs](architecture.md#how-harbor-differs-from-a-traditional-kernel) | 2       |
| 3   | Where it is going, and what is actually done | [roadmap § H1 working order](roadmap.md#next-working-order-post-h1-hw-stamp)                                                    | 1       |
| 4   | Why any of it should be believed             | [verification](verification.md) — **index only**, do not read it through                                                        | —       |

Depth after that: [`architecture.md`](architecture.md) (normative model),
[`vision.md`](vision.md) (product shape and use cases),
[`stack.md`](stack.md) (what it is built with),
[`SECURITY.md`](../SECURITY.md) (threat model),
[`CONTRIBUTING.md`](../CONTRIBUTING.md) (how to add work),
[`adr/`](adr/README.md) (why the boundaries are where they are).

## By goal

| Goal                              | Start                              | Then                                                                                                                                            |
| --------------------------------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| Understand a term                 | [glossary](glossary.md)            | the owning document named in its row                                                                                                            |
| Know the toolchain / platform     | [stack](stack.md)                  | [porting](porting.md), [blobs](blobs.md)                                                                                                        |
| Build and boot                    | [README](../README.md)             | [boot-chain](boot-chain.md), [hardware](hardware.md)                                                                                            |
| Completeness + product path (K/P) | [roadmap](roadmap.md)              | [ADR-0026](adr/0026-kernel-and-product-completeness.md), [vision](vision.md)                                                                    |
| Understand the agent model        | [architecture](architecture.md)    | [differs §](architecture.md#how-harbor-differs-from-a-traditional-kernel), [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) |
| Product vision / use cases        | [vision](vision.md)                | architecture, SECURITY                                                                                                                          |
| Authority and threats             | [SECURITY](../SECURITY.md)         | architecture agent model                                                                                                                        |
| Verify a claim                    | [verification](verification.md)    | linked transcript or gate                                                                                                                       |
| Port ISA/board                    | [porting](porting.md)              | [arch-contract](arch-contract.md)                                                                                                               |
| Structural decision               | [adr/](adr/README.md)              | the linked ADR                                                                                                                                  |
| Extend the tree                   | [CONTRIBUTING](../CONTRIBUTING.md) | [scripts map](../scripts/README.md)                                                                                                             |

## Ownership and status vocabulary

| Document                                                | Owns                                                                     | Does not own                                               |
| ------------------------------------------------------- | ------------------------------------------------------------------------ | ---------------------------------------------------------- |
| `README.md`                                             | Public story, status snapshot, quick start                               | Full K/P tables, evidence transcripts, the mission wording |
| [`roadmap.md`](roadmap.md)                              | **H0–H2 outcomes, H1 order, K/P status** (SSOT)                          | Per-track design ADRs, the mission wording                 |
| `docs/architecture.md`                                  | Normative model **as it is today**, layering                             | Live K/P table copies, foundation history                  |
| `docs/vision.md`                                        | **The mission sentence**, product shape, horizons, use cases, audience   | Silicon status claims                                      |
| [`docs/stack.md`](stack.md)                             | Language, target, toolchain, features, verification stack                | Status, model, evidence                                    |
| [`docs/glossary.md`](glossary.md)                       | One-line definitions + which document owns each term                     | Any definition of record                                   |
| [`docs/foundation-history.md`](foundation-history.md)   | M0–M8 milestone record, closed slices, foundation findings               | Live planning, current status                              |
| [ADR-0026](adr/0026-kernel-and-product-completeness.md) | Completeness as goal                                                     | Per-track design                                           |
| `SECURITY.md`                                           | Threat model, authority surface, residuals                               | Roadmap ordering                                           |
| `docs/verification.md`                                  | Gates, transcripts, blind spots                                          | Normative design                                           |
| `docs/adr/*.md`                                         | Immutable structural decisions (`amended:` reconciliations per ADR-0058) | Live dashboard                                             |
| `docs/reviews/*.md`                                     | Dated findings                                                           | Current truth after later fixes                            |
| `docs/design/*.md`                                      | Design contracts                                                         | Completion claims without evidence                         |
| `scripts/README.md`                                     | Script taxonomy                                                          | Kernel architecture                                        |

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
  a64, agentstore, asid, budget, bump, cap, capslots, cpuid, delay, density, display, durable, font8x8, frame, gic, heap,
  ipc, irqcap, irqtable, irqwait, layout, manifest, naming, paging, parktime, poll, preempt, prog, reply, reset, ring, rng,
  runqueue, rxline, spi, storage, syscall, taskcap, tasks, textgrid, timer, uart, wake
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
  naming/         EL1 name registry (ADR-0035)
  taskcap/        task capabilities for peer transfer (ADR-0054)
  storage/        EL1 keyed blob store (ADR-0036)
  durable/        durable section store (ADR-0045)
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

| Layer                | State (2026-08-09)                                                                                               |
| -------------------- | ---------------------------------------------------------------------------------------------------------------- |
| **H0 foundation**    | **done (HW)** on Pi 4B (M0–M8 + parked cancel)                                                                   |
| **H1 entry + depth** | **paid (HW)** — serial stamp 2026-08-08 (see verification)                                                       |
| **H1 next**          | P3\|P4 if composition · SD power-cycle                                                                           |
| **H2**               | K7 first slice done (HW); K4 budget + EL0 preemption done (HW, ADR-0064), same-EL in design; K8 design; SMP open |
| **Standing watch**   | [#14](https://github.com/gianlucamazza/harbor-kernel/issues/14) SpiDevice / ADR-0020                             |

## Decision records and reviews

[`adr/README.md`](adr/README.md) — ADR lifecycle. Accepted ADRs are immutable;
change requires a successor — except narrow reconciliation amendments under
[ADR-0058](adr/0058-adr-amendments-and-mutation-freshness.md)'s `amended:`
convention. [`reviews/`](reviews/) — dated findings, not live
status. Active work: [roadmap](roadmap.md); foundation history:
[foundation-history.md](foundation-history.md).

## Documentation checks

```bash
make doc-claims
make doc-symbols
make xrefs
```

These run inside `make check`. They compare sets, links, and paths — not whether
prose is conceptually honest.
