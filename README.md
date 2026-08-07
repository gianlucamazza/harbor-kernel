# Harbor

**A verified Rust agent-based microkernel and product OS for Raspberry Pi 4**
(`harbor-kernel`).

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![CI](https://github.com/gianlucamazza/harbor-kernel/actions/workflows/ci.yml/badge.svg)](https://github.com/gianlucamazza/harbor-kernel/actions)

## What it is

Harbor is building an operating system where software runs as **agents**:
isolated programs that talk only through **messages** and hold only the
**capabilities** they were granted. The goal is to **finish** that microkernel
and the product services on top of it — not to stay a permanent demo, and not
to clone Linux.

Important boundaries are demonstrated with host tests, QEMU gates, or Raspberry
Pi 4B evidence ([`docs/verification.md`](docs/verification.md)). “Done” means
more than “it compiles.”

**Agent** here is the isolation unit, not an LLM product. Tool-limited software
can live *inside* an agent later; Harbor is not a chat/runtime framework.

Policy on completeness: [ADR-0026](docs/adr/0026-kernel-and-product-completeness.md).  
Product shape and use cases: [`docs/vision.md`](docs/vision.md).

## How it works

In a conventional OS a process is usually both isolated and scheduled. In Harbor
an agent is a **pair**:

- an EL0 **program** — private address space, capability slots only;
- an EL1 **driver task** — what the scheduler actually runs, including the
  session loop that enters and resumes the program.

Authority is structural: user code names a **slot index**, never a raw kernel
handle. Agents can be described as **manifest data** (image + grants). Drivers
can be agents with page-sized device maps. Faults end the session; the creator
decides the task’s fate.

Full contrast:
[architecture § How Harbor differs](docs/architecture.md#how-harbor-differs-from-a-traditional-kernel).

## Where we are

| | |
| --- | --- |
| **Foundation** | **Complete on Pi 4B** (2026-08-07): tasks, IPC/caps, EL0, PL011 driver-agent, slot ABI, blocking recv, manifest loader, console endpoint + beacon, supervisor cancel of parked waits |
| **Goal** | Complete microkernel (**K**) and product OS (**P**) — [roadmap](docs/architecture.md#completeness-roadmap) |
| **Not yet** | Preemption/budget, IRQ as first-class wait, timeout/auto-reap, cap transfer, SMP, external load, multi-agent product, storage, network, … |

**What works today (short list):** cooperative tasks; message IPC; EL0 agents
with private memory; least-privilege console (denied by default); PL011 as a
contained driver agent; product beacon via the console endpoint.

| Area | State |
| --- | --- |
| Platform | Single-core AArch64, Pi 4B, early MMU, W^X, heap, guarded stacks |
| Execution | Cooperative only — preemption/SMP **open** |
| Authority | Slot caps, refuse accounting, cancel blocked wait — transfer/timeout **open** |
| Product OS | Beacon composition; broader services **open** |
| Verification | 293 host tests, model checks, Miri, QEMU and hardware stamps |

Evidence index: [`docs/verification.md`](docs/verification.md).

## What we are not

| Out of model | Why |
| --- | --- |
| Linux / POSIX / glibc | Different ABI and ambient-authority world |
| “Intentionally incomplete forever” | Gaps are **open work**, not identity |
| Multi-tenant cloud hypervisor | Separate problem; needs its own design if ever |
| Hiding platform firmware | Blobs stay explicit ([`docs/blobs.md`](docs/blobs.md)) |

## Quick start

Toolchain: [`rust-toolchain.toml`](rust-toolchain.toml) · target
`aarch64-unknown-none-softfloat`.

```bash
make              # release kernel8.img
make test         # host tests
make qemu         # boot in QEMU
make check        # fmt-check test no-simd no-early-exclusives no-static-mut irq-scope boot-check bringup-builds debug-display-builds debug-builds board-guard product-builds product-boot-check miri doc-claims doc-symbols layering arch-board-free shellcheck xrefs, then clippy
```

On a Pi 4B (FAT boot partition + 3.3 V USB-serial):

```bash
make blobs
make deploy SD_MOUNT=/run/media/$USER/boot
make serial SERIAL_DEV=/dev/ttyUSB0
```

Optional SPI TFT status panel: `FEATURES=debug-display` — see
[`docs/hardware.md`](docs/hardware.md). UART remains the primary console.

## Documentation

| I want to… | Read |
| --- | --- |
| Navigate all docs | [`docs/README.md`](docs/README.md) |
| Architecture, layering, roadmaps | [`docs/architecture.md`](docs/architecture.md) |
| Product vision and use cases | [`docs/vision.md`](docs/vision.md) |
| Threat model and authority | [`SECURITY.md`](SECURITY.md) |
| What is actually proven | [`docs/verification.md`](docs/verification.md) |

Hardware and boot: [`docs/boot-chain.md`](docs/boot-chain.md),
[`docs/hardware.md`](docs/hardware.md). Decisions: [`docs/adr/`](docs/adr/README.md).

## Contributing

Boundary changes need an ADR first ([ADR-0001](docs/adr/0001-multi-role-analysis.md)).
Before calling work complete: `make check`. Keep status in the owning doc,
evidence in verification, history in ADRs/reviews — see
[`docs/README.md`](docs/README.md).

## License

MIT ([`LICENSE-MIT`](LICENSE-MIT)) or Apache-2.0
([`LICENSE-APACHE`](LICENSE-APACHE)), at your option. Platform blobs: upstream
licenses in [`docs/blobs.md`](docs/blobs.md).
