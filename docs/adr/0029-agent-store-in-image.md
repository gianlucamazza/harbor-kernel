---
id: 0029
title: Agent store lives in the kernel image section (placement for H1)
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0021, 0027]
---

# ADR-0029: Image-resident agent store placement

## Acceptance status

**Accepted** (2026-08-07). Completes placement for [ADR-0027](0027-h1-external-agent-store.md)
so the product path is the **same on QEMU and Pi**, with no fixed-PA lab-only
path and no FAT file that the kernel never reads.

## Context

ADR-0027 defined the wire format and a fixed physical address for QEMU's
`loader` device. That left silicon with a store the firmware does not place,
and a deploy step that copied `agents.bin` onto FAT as dead weight — a path that
looked like a product feature while only the emulator exercised it.

Harbor does not yet have an in-kernel filesystem. The honest external path that
works everywhere the image boots is:

1. Reserve a page-aligned **`.agent_store`** section in the linked image.
2. Host tool packs `agents.bin` and **injects** it into that section of the raw
   `kernel8*.img` after `objcopy`.
3. The loader parses that section; empty zeros → builtin fallback.

Composition still changes **without** recompiling agent programs into Rust
`const` tables: only the host pack+inject runs.

## Decision

| Item | Value |
| --- | --- |
| Section | `.agent_store` (linker + `#[link_section]`) |
| Capacity | 16 KiB (`AGENT_STORE_CAPACITY`) |
| Inject | `scripts/inject-agent-store.py --elf … --image … --store …` |
| Product gate | inject after product `objcopy`; QEMU boots that image with **no** `-device loader` |
| Oracle | section left zero → `loader: builtin` |
| Fixed PA `0x10000000` | **withdrawn** as a load path |

Log line: `loader: store n=… image` vs `loader: builtin`.

## Consequences

### Positive

- One load path on every machine that boots `kernel8.img`.
- No QEMU-only product gate.
- No deploy of a store the kernel ignores.

### Negative

- Capacity is fixed at link time (raise the constant + rebuild to grow).
- Inject must stay in the product image pipeline or the product falls back to
  builtin (gate catches that).

### Gates

| Reversal | Gate |
| --- | --- |
| Product without inject | `product-boot-check` requires `loader: store n=2 image` |
| Oracle without zeros | `boot-check` requires `loader: builtin` |
| Section missing | inject tool fails; product-builds fails |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Keep fixed-PA only | Pi never places it; dual story is debt |
| Ship `agents.bin` on FAT for later | Kernel never opens FAT; false product surface |
| Full FS reader first | Still out of scope; wrong size for this gap |
