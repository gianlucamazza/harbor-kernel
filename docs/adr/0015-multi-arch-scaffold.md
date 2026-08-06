---
id: 0015
title: Multi-arch scaffold — cfg facade, board features, per-ISA boot/link
status: accepted
date: 2026-08-05
accepted: 2026-08-05
---

# ADR-0015: Multi-arch scaffold (structure ready, one product target)

## Acceptance status

**Accepted** (2026-08-05). Product identity remains AArch64 + Raspberry Pi 4
Model B ([ADR-0007](0007-project-identity-harbor-kernel.md)). This ADR records
how ports will land later without rewriting policy.

**Implemented with this decision:**

- `crate::arch` facade selected by `#[cfg(target_arch = "…")]` + `compile_error!`
- Board selected by Cargo feature `board-rpi4` (default) + `crate::bsp::board`
- Boot entry and linker script under `src/arch/aarch64/`
- Facade isolation enforced by `make layering` (rule 10 in
  [`architecture.md`](../architecture.md))
- Contract and checklist: [`arch-contract.md`](../arch-contract.md),
  [`porting.md`](../porting.md)

## Context

Harbor was multi-layer (`arch` / `bsp` / `drivers` / policy) but mono-path:

- `arch/mod.rs` always re-exported `aarch64` with no `cfg`
- `boot.s` and `link.ld` lived at repo/src root as if universal
- Policy could import `crate::bsp::rpi4` (one leak in `status`)
- Layering saw only the first `crate::` segment, so `crate::arch::aarch64::…`
  would have passed the gate

A full second ISA is out of product scope. Shipping empty `riscv64` stubs or
`dyn Arch` traits would add maintenance without a runner. The gap that *did*
matter was structural: a future port should be “sibling module + board feature
+ target”, not a cross-cutting rename of the tree.

## Decision

1. **Compile-time ISA selection only** via `target_arch`. No runtime arch
   switch, no `dyn Arch`, no megatrait for the whole CPU surface.
2. **Single facade** — code outside `src/arch/` imports only
   `crate::arch::{cpu, mmu, …}`. ISA modules are private to the facade.
3. **Board via Cargo feature** — `board-*` features; default `board-rpi4`.
   Policy uses `crate::bsp::board` only.
4. **Per-ISA artefacts** — `boot.s`, `link.ld`, exception vectors live under
   `src/arch/<isa>/`.
5. **One supported product combo** — AArch64 + Pi 4; Makefile `ARCH`/`BOARD`
   reject other values until a port lands.
6. **Document the contract** — [`arch-contract.md`](../arch-contract.md) lists
   modules the facade must re-export; [`porting.md`](../porting.md) is the
   operational checklist.

## Consequences

### Positive

- Ports add a tree and wire `cfg` / features; policy and most drivers stay put.
- Gates catch facade leaks the way rules 1–4 catch layering inversions.
- Product scope stays honest: multi-arch *ready*, not multi-arch marketed.

### Negative / debt

- Naming in the facade is still AArch64-shaped (`el0`, TTBR language in docs
  and `mm::aspace`). A real non-AArch64 port will rename or alias some of that
  — recorded in the contract as known debt, not fixed preemptively.
- Drivers (GICv2, PL011, RNG200, BCM SPI) remain SoC-family specific; a new
  board may need new drivers even on the same ISA.
- `build.rs` / `.cargo/config.toml` still hardcode the aarch64 linker path
  until a second target exists (then per-target rustflags).

### Gates that catch reversal

| Reversal | Gate |
| -------- | ---- |
| Policy imports `crate::arch::aarch64` | `make layering` |
| Policy imports `crate::bsp::rpi4` | `make layering` |
| Boot/link leave the ISA tree without updating build | link fails or stale map; `build.rs` `rerun-if-changed` |
| Unsupported `target_arch` | `compile_error!` in `arch/mod.rs` |
| No board feature | `compile_error!` in `bsp/mod.rs` |
| Behaviour regression | `make check` (incl. QEMU boot-check) |

## Alternatives rejected

| Alternative | Why not |
| ----------- | ------- |
| `dyn Arch` / trait object CPU | Wrong cost model; arch is monomorphized and cfg-selected in modern Rust kernels |
| Second ISA skeleton (compile-only riscv/x86) | No product runner; bitrots; dilutes CI signal |
| Workspace crate per arch | Overkill for one product binary |
| Rename EL0/TTBR everywhere now | Large churn with zero second consumer |
| Multi-board without features (`if board ==`) | Runtime board detect is wrong for bare metal; use features |

## Related

- [0007](0007-project-identity-harbor-kernel.md) — product identity Pi 4
- [0003](0003-early-mmu.md) — early MMU from boot path
- [architecture.md](../architecture.md) — layering rule 10
- [arch-contract.md](../arch-contract.md), [porting.md](../porting.md)
