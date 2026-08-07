# Harbor documentation

This is the map for the repository’s documentation. Start with the README for
the public project story; use this page when you need to decide which document
owns a fact or which technical path to follow.

## Mission and current model

- [`../README.md`](../README.md) — public introduction, mission, current status,
  quick start and reading paths.
- [`architecture.md`](architecture.md) — normative architecture, layer rules,
  agent model, milestones and ordered roadmap.
- [`vision.md`](vision.md) — long-term OS direction and use cases (aspirational;
  not a status claim).
- [`../SECURITY.md`](../SECURITY.md) — threat model, TCB, syscall authority
  surface and residual risks.

The current mental model is: an **agent** is an EL1 driver task paired with an
EL0 program; the program runs in a private address space and uses explicitly
granted capability slots; the driver is the schedulable half; messages are the
interaction boundary. That pairing is why Harbor does not read like a
traditional process OS — see
[architecture § How Harbor differs](architecture.md#how-harbor-differs-from-a-traditional-kernel).
The architecture document owns the detailed model and the contrast table.

## Choose a path

| Goal | Start here | Then read |
| --- | --- | --- |
| Build and boot the kernel | [`../README.md`](../README.md) | [`boot-chain.md`](boot-chain.md), [`hardware.md`](hardware.md) |
| Understand memory protection | [`mmu.md`](mmu.md) | [`arch-contract.md`](arch-contract.md), [`verification.md`](verification.md) |
| Understand interrupts and idle | [`interrupts.md`](interrupts.md) | [`architecture.md`](architecture.md) |
| Understand agents and authority | [`architecture.md`](architecture.md) | [`../SECURITY.md`](../SECURITY.md), relevant ADRs |
| See why Harbor is not a traditional process OS | [`architecture.md` § differs](architecture.md#how-harbor-differs-from-a-traditional-kernel) | [`../SECURITY.md`](../SECURITY.md), [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) |
| See the long-term OS vision and future use cases | [`vision.md`](vision.md) | [`architecture.md`](architecture.md), [`../SECURITY.md`](../SECURITY.md) |
| Verify a claim | [`verification.md`](verification.md) | the linked transcript or gate |
| Add an ISA or board | [`porting.md`](porting.md) | [`arch-contract.md`](arch-contract.md) |
| Understand firmware dependencies | [`boot-chain.md`](boot-chain.md), [`blobs.md`](blobs.md) | [`hardware.md`](hardware.md) |
| Understand a structural decision | [`adr/README.md`](adr/README.md) | the linked ADR |

## Ownership and status vocabulary

Each kind of document has one job:

| Document | Owns | Does not own |
| --- | --- | --- |
| `README.md` | Public orientation and quick start | Complete implementation inventory or evidence transcripts |
| `docs/architecture.md` | Current architecture, invariants and roadmap | Historical review narrative; product vision |
| `docs/vision.md` | Long-term OS direction, horizons, future use cases | Milestone status, evidence, operational threat model |
| `SECURITY.md` | Threat model and authority claims | General project roadmap |
| `docs/verification.md` | Test methodology, gates, transcripts and blind spots | Normative architectural decisions |
| `docs/adr/*.md` | Accepted/proposed/superseded structural decisions | Current milestone dashboard outside the decision context |
| `docs/reviews/*.md` | Dated findings and review outcomes | Current truth after later changes |
| `docs/design/*.md` | Design proposals and implementation contracts | Completion claims unless explicitly verified |

Use these status labels precisely:

- **implemented** — present in the current checkout;
- **done (QEMU)** — exercised by the QEMU gate;
- **done (HW)** — observed on Raspberry Pi 4B silicon;
- **proposed** — design exists but is not accepted or complete;
- **open** — intentionally unfinished or awaiting a decision;
- **historical** — retained because it records what was true at a past date.

When a fact changes, update its owning document and link to evidence. Do not
copy a full status table into another document merely for convenience.

## Code layout

The following map is checked against the source tree. It describes ownership,
not every symbol.

```
crates/kernel-core/  pure logic, host-tested — no MMIO, no assembly:
  a64, bump, cap, delay, display, font8x8, frame, gic, heap, ipc,
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

[`adr/README.md`](adr/README.md) is the ADR index and lifecycle contract.
Accepted ADRs are immutable; a changed decision gets a successor and the old
one becomes superseded. [`reviews/`](reviews/) contains dated analysis and
findings, not an alternative source of current status. The active roadmap
remains in [`architecture.md#roadmap`](architecture.md#roadmap).

## Documentation checks

The documentation is part of the verification surface:

```bash
make doc-claims
make doc-symbols
make xrefs
```

`make check` runs these alongside code, build, QEMU and layering gates. The
checks can compare sets, links and paths; they cannot judge whether prose is
conceptually honest, so claims about meaning and evidence still require review.

