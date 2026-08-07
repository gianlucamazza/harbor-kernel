# Harbor

**A verified Rust microkernel laboratory for Raspberry Pi 4** — package
`harbor-kernel`.

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![CI](https://github.com/gianlucamazza/harbor-kernel/actions/workflows/ci.yml/badge.svg)](https://github.com/gianlucamazza/harbor-kernel/actions)

Harbor is an experiment in building an **agent-based microkernel**: isolated
units with private memory and explicit authority, communicating through
messages and capabilities. The name describes the intended system — a
protected place where bounded components can operate and talk over controlled
channels ([ADR-0007](docs/adr/0007-project-identity-harbor-kernel.md)).

This is a **lab kernel, not a production operating system**. Its purpose is to
make kernel boundaries concrete and testable on real hardware. A claim is not
considered complete merely because the code compiles: it needs a host test,
QEMU gate, or Raspberry Pi evidence, with blind spots recorded in
[`docs/verification.md`](docs/verification.md).

## What Harbor is today

Harbor is a single-core, bare-metal AArch64 kernel written in Rust (`no_std`)
for the Raspberry Pi 4 Model B. It boots through the platform firmware and
currently provides:

- cooperative EL1 tasks with guarded stacks;
- IPC through mailboxes and unforgeable capability handles;
- EL0 agents in private address spaces;
- authority named by capability-slot index rather than by a forgeable handle;
- agents represented as manifest data and created by a loader;
- blocking message receive, where an agent can park and a peer can wake it;
- an EL1 console server and a product beacon agent communicating through the
  same endpoint model.

The kernel is intentionally incomplete. Agents are not preempted, and an IRQ
is not yet a first-class wait source for an agent. A parked agent has no timeout
or reclamation policy. These are known availability limitations, not hidden
features. The current architecture and ordered roadmap live in
[`docs/architecture.md`](docs/architecture.md).

## Mission and boundaries

Harbor exists to answer a focused question:

> Can a small Rust kernel make isolation, authority, message passing and
> verification visible enough that each boundary can be inspected, tested and
> demonstrated on silicon?

The project therefore values explicit mechanisms and evidence over feature
count. It does not currently target Linux/POSIX compatibility, preemption,
SMP, a high-half kernel, production ASIDs, USB host support or a full
framebuffer. Those would require their own architectural decisions.

### Why it looks different

In a traditional kernel a process is usually both isolated and schedulable.
In Harbor an **agent is a pair**: a cooperative EL1 **driver task** (what the
scheduler runs) and an EL0 **program** (private address space, slot-indexed
capabilities). Authority is structural, agents can be described as manifest
data, and “done” for a boundary means evidence on hardware — not only that a
feature compiles. The contrast is spelled out in
[architecture: How Harbor differs from a traditional kernel](docs/architecture.md#how-harbor-differs-from-a-traditional-kernel).

Security and authority claims, including their residual risks, are documented
in [`SECURITY.md`](SECURITY.md). Platform and hardware assumptions are in the
[documentation index](docs/README.md).

## Current status

The foundation through **M8** is stamped **done on Raspberry Pi 4B**: console
endpoint, product beacon, and `SYS_PUTC` retirement included
([evidence](docs/verification.md#hardware-evidence-m8-console-endpoint-closed-on-silicon-2026-08-07)).
Parked-task **visibility and supervisor cancel** are stamped **done on Pi 4B**
(ADR-0024/0025,
[evidence](docs/verification.md#parked-task-visibility-and-cancel-closed-on-silicon-adr-0024--0025-2026-08-07));
timeout and auto-reap on last send drop remain named non-goals.

| Area | Current state |
| --- | --- |
| Boot and memory | EL2→EL1 boot, early MMU, W^X kernel map, heap, guarded stacks and runtime page-table operations |
| Execution | Cooperative EL1 tasks; no preemption or SMP |
| IPC and authority | Mailboxes, capability checks, slot-indexed EL0 authority, cancel of blocked waits |
| Agents | Private address spaces, multi-SVC sessions, fault termination, manifest loader and blocking receive |
| Drivers | PL011 driver-agent path with narrow mapping, RX ownership and restoration on kill |
| Console | EL1 endpoint server plus product beacon; transitional `SYS_PUTC` removed in M8 |
| Verification | 290 host tests, bounded model checks, Miri, build gates, QEMU boot checks and fault probes on hardware |

The ordered roadmap, done-when criteria and evidence links are maintained in
[`docs/architecture.md#roadmap`](docs/architecture.md#roadmap), rather than
duplicated here.

## Quick start

The repository uses the toolchain in [`rust-toolchain.toml`](rust-toolchain.toml)
and targets `aarch64-unknown-none-softfloat`.

```bash
make              # build target/aarch64-unknown-none-softfloat/release/kernel8.img
make test         # host tests
make qemu         # boot the image in QEMU
make check        # fmt-check test no-simd no-early-exclusives no-static-mut irq-scope boot-check bringup-builds debug-display-builds debug-builds board-guard product-builds product-boot-check miri doc-claims doc-symbols layering arch-board-free shellcheck xrefs, then clippy
```

For a Raspberry Pi run, fetch the verified firmware blobs, deploy to a FAT boot
partition and use a 3.3 V USB-serial adapter:

```bash
make blobs
make deploy SD_MOUNT=/run/media/$USER/boot
make serial SERIAL_DEV=/dev/ttyUSB0
```

The optional SPI TFT is a debug status surface, not the primary console. Build
and deploy it consistently with `FEATURES=debug-display`. Hardware wiring and
the safety constraints are in [`docs/hardware.md`](docs/hardware.md).

## Where to read next

[`docs/README.md`](docs/README.md) is the documentation map. The shortest useful
paths are:

| If you want to… | Read… |
| --- | --- |
| Understand the architecture and roadmap | [`docs/architecture.md`](docs/architecture.md) |
| See why Harbor is not a traditional process OS | [architecture § how it differs](docs/architecture.md#how-harbor-differs-from-a-traditional-kernel) |
| Understand authority, isolation and threats | [`SECURITY.md`](SECURITY.md) |
| See what is actually verified | [`docs/verification.md`](docs/verification.md) |
| Follow boot and hardware setup | [`docs/boot-chain.md`](docs/boot-chain.md), [`docs/hardware.md`](docs/hardware.md) |
| Understand MMU or interrupt invariants | [`docs/mmu.md`](docs/mmu.md), [`docs/interrupts.md`](docs/interrupts.md) |
| Port the scaffold | [`docs/porting.md`](docs/porting.md), [`docs/arch-contract.md`](docs/arch-contract.md) |
| Understand an architectural choice | [`docs/adr/README.md`](docs/adr/README.md) |

## Repository map

The map is intentionally compact: it explains ownership, while the
documentation index explains how to navigate the material.

```
crates/kernel-core/  host-tested pure logic: authority, ipc, syscall, tasks,
                     manifest, memory, hardware maths, data structures, models
src/                 AArch64 facade, BSP, IRQ/drivers, bootstrap, scheduler,
                     agents, memory, console and panic/runtime policy
boot/                Raspberry Pi firmware configuration
docs/                architecture, security links, hardware, verification,
                     porting, ADRs and historical reviews
scripts/              build, layering, documentation and QEMU gates
```

The detailed module inventory remains in the architecture documentation and is
validated by the documentation gates.

## Contributing

There is no external contributor process yet. Changes that move a kernel
boundary need the relevant ADR and, for milestone work, the multi-role review
discipline described by [ADR-0001](docs/adr/0001-multi-role-analysis.md).

Before describing a change as complete, run:

```bash
make check
```

Keep current behavior in the owning document, evidence in the verification
record, and historical decisions in their original ADR or review. See
[`docs/README.md`](docs/README.md) for the documentation conventions.

## License

MIT ([`LICENSE-MIT`](LICENSE-MIT)) or Apache-2.0
([`LICENSE-APACHE`](LICENSE-APACHE)), at your option. Platform blobs remain
under their upstream licenses; see [`docs/blobs.md`](docs/blobs.md).
