# Harbor

**A verified Rust agent-based microkernel and product OS for Raspberry Pi 4**
(`harbor-kernel`).

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![CI](https://github.com/gianlucamazza/harbor-kernel/actions/workflows/ci.yml/badge.svg)](https://github.com/gianlucamazza/harbor-kernel/actions)

## Mission

<!-- mission:begin -->

> Harbor is an OS where software arrives as **agents**, authority arrives as
> **grants**, and every boundary can be **shown to hold** — and the project
> **finishes** that OS, mechanism by mechanism and service by service.

<!-- mission:end -->

One sentence, one owner: [`docs/vision.md`](docs/vision.md). Every other
document quotes it rather than rephrasing it, and `make doc-claims` fails if a
copy drifts. It is a **goal** — the kernel and the product OS are not finished
today.

Concretely: software runs as isolated programs that talk only through
**messages** and hold only the **capabilities** they were granted, and
important boundaries are demonstrated with host tests, QEMU gates or Raspberry
Pi 4B evidence ([`docs/verification.md`](docs/verification.md)) — “done” means
more than “it compiles.”

**Agent** here is the isolation unit, **not an LLM product**. Tool-limited
software can live *inside* an agent later; Harbor is not a chat/runtime
framework. Unfamiliar vocabulary: [`docs/glossary.md`](docs/glossary.md).

## Objectives

| Horizon | Product outcome | State |
| --- | --- | --- |
| **H0 — Foundation** | Boundary lab on Pi 4B: tasks, caps, EL0, PL011 driver-agent, blocking recv, console + beacon, cancel | **done (HW)** |
| **H1 — Composition / appliance OS** | A multi-agent product you can compose and load, with an early device and supervisor story | **in progress** — first slices done (QEMU) |
| **H2 — Boundary OS** | Fair execution, denser agents, production isolation, multi-core, remaining platform paths | later |

Completeness of the kernel (**K**) and the product OS (**P**) is the goal, not a
permanent demo: [ADR-0026](docs/adr/0026-kernel-and-product-completeness.md).
**Per-track status lives in [`docs/roadmap.md`](docs/roadmap.md) only** — this
page carries a snapshot, never a second table. Product shape and use cases:
[`docs/vision.md`](docs/vision.md).

## Technology stack

| Layer | Choice |
| --- | --- |
| Language | Rust, edition 2024, `no_std`, **zero dependencies** |
| Target | `aarch64-unknown-none-softfloat` (pinned toolchain, `panic = "abort"`) |
| Platform | Raspberry Pi 4B / BCM2711, AArch64, **single core** for now |
| Build | `make` over `cargo`; `kernel8.img` at `0x80000`, EL2 → EL1h |
| Model | Cooperative tasks · slot-indexed capabilities · agent = EL1 driver task + EL0 program |
| Evidence | Host tests · Miri · QEMU oracles in `make check`; Pi 4B serial stamps by hand |

Full stack, including what is deliberately **not** in it:
[`docs/stack.md`](docs/stack.md).

## Who this is for

**For:** people building or studying isolation and capability systems on bare
metal, and anyone who wants a composable appliance OS on a Pi 4 rather than a
distro to strip down.

**Not for:** Linux/POSIX or driver-compatibility work, cloud hypervisors, or
LLM/agent chat frameworks. Those are out of model, not backlog
([`docs/vision.md`](docs/vision.md#who-this-is-for)).

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

Snapshot, 2026-08-07 — status of record is [`docs/roadmap.md`](docs/roadmap.md).

| | |
| --- | --- |
| **Foundation** | **Complete on Pi 4B**: tasks, IPC/caps, EL0, PL011 driver-agent, slot ABI, blocking recv, manifest loader, console endpoint + beacon, supervisor cancel of parked waits |
| **H1 first slices (QEMU)** | Store (**K6**), wait-on-IRQ (**K1**), auto-reap (**K2**), revoke (**K3**), RNG (**K9**), supervisor (**K10**), multi-agent (**P1**), names (**P5**), compose (**P6**) |
| **H1 next** | Storage — [roadmap](docs/roadmap.md) |
| **Not yet (later)** | IRQ preemption, SMP, ASID residuals (TTBR1/HW stamp), full product net/display depth, … |

**What works today (short list):** cooperative tasks; message IPC; EL0 agents
with private memory; least-privilege console; PL011 driver-agent; product
composition via injected store (beacon + chirp); EL1+EL0 IRQ wait; last-SEND-hold
auto-reap (ephemeral channels); channel revoke (stale CapId refused).

| Area | State |
| --- | --- |
| Platform | Single-core AArch64, Pi 4B, early MMU, W^X, heap, guarded stacks |
| Execution | Cooperative only — preemption/SMP **open** |
| Authority | Slot caps, cancel, auto-reap, revoke, supervisor reap — transfer/timeout/creator-exit cascade **open** |
| Product OS | Multi-agent store composition (QEMU); broader services **open** |
| Verification | 348 host tests, model checks, Miri, QEMU and hardware stamps |

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
| Navigate all docs (5-minute path) | [`docs/README.md`](docs/README.md) |
| Decode the vocabulary | [`docs/glossary.md`](docs/glossary.md) |
| Know what it is built with | [`docs/stack.md`](docs/stack.md) |
| Completeness tracks (K/P) | [`docs/roadmap.md`](docs/roadmap.md) |
| Architecture and layering | [`docs/architecture.md`](docs/architecture.md) |
| Product vision and use cases | [`docs/vision.md`](docs/vision.md) |
| Threat model and authority | [`SECURITY.md`](SECURITY.md) |
| What is actually proven | [`docs/verification.md`](docs/verification.md) |
| How the foundation was closed | [`docs/foundation-history.md`](docs/foundation-history.md) |
| How to contribute | [`CONTRIBUTING.md`](CONTRIBUTING.md) |

Hardware and boot: [`docs/boot-chain.md`](docs/boot-chain.md),
[`docs/hardware.md`](docs/hardware.md). Decisions: [`docs/adr/`](docs/adr/README.md).
Scripts layout: [`scripts/README.md`](scripts/README.md).

## License

MIT ([`LICENSE-MIT`](LICENSE-MIT)) or Apache-2.0
([`LICENSE-APACHE`](LICENSE-APACHE)), at your option. Platform blobs: upstream
licenses in [`docs/blobs.md`](docs/blobs.md).
