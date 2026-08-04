---
id: 0003
title: MMU enabled before any Rust runs
status: proposed
date: 2026-08-04
---

# ADR-0003: MMU enabled before any Rust runs

## Context

The board loaded the kernel and stayed silent: the ACT LED lit while the card was
read and then went out — the signature of a _successful_ load — and no output.

The cause was an `AtomicBool::swap` in `console::acquire`, the first statement of
`bootstrap::run`. An atomic read-modify-write compiles to an `LDXR`/`STXR` pair,
and with translation off every access is Device-nGnRnE, where exclusives make no
forward progress on a Cortex-A72: the retry loop spins forever. No fault, no
output.

QEMU booted the image without trouble, because TCG's exclusive monitor ignores
memory attributes. **Emulation cannot catch this class at all.**

The project already had this lesson written down as an M1 gotcha. It was
withdrawn on incomplete reasoning — "it only applies before M2" — and
reintroduced the same day, with the note in plain sight.

## Decision

`boot.s` enables translation **before calling `kernel_main`**, using a coarse
identity map evaluated at compile time (`arch::mmu::EARLY_L1`): three 1 GiB
blocks of RAM plus the device window.

The point is not the map — it is that **no kernel code runs without memory
attributes**. The fine-grained W^X map becomes a `TTBR0` switch
(`mmu::activate`).

This is the arm64 Linux sequence (`__create_page_tables` + `__enable_mmu` in
`head.S`, before `start_kernel`), and that of ARM Trusted Firmware, Zephyr and
seL4.

## Consequences

**Positive** — the window in which different rules apply no longer exists, so it
cannot be forgotten; atomics are legal everywhere; caches are on from the start;
and switching a live map is simpler than a cold enable (table writes and walker
reads go through the same caches, so a barrier suffices where otherwise
everything would need invalidating).

**Negative** — the initial map is RWX over 3 GiB until `activate` replaces it.
That is necessary: without attributes you cannot even reach the console. If
`activate` fails you stay there, which is a declared risk (finding F14).

## Alternatives considered

| Alternative                             | Why not                                                                                               |
| --------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| Keep the window, forbid RMW inside it   | That is the rule that has already been forgotten once, by the person who wrote it                     |
| `SyncCell` instead of atomics there     | Correctness by assumption rather than by construction: it breaks on the first secondary core          |
| Build the fine map directly in `boot.s` | It needs the layout from the linker and an error path — that is, a console — which does not exist yet |

## The gate that protects this decision

`scripts/check-pre-mmu-path.sh` derives the `_start` → gate path from the image
and fails if an exclusive appears or if the path grows. **Seen red** by planting
a `fetch_add` called from `_start` before the gate.

## When to revisit

At the first secondary core: each will have to run its own `early_mmu_enable`
before touching any shared state.

## References

`src/boot.s`, `src/arch/aarch64/mmu.rs`, [`mmu.md`](../mmu.md),
[`verification.md`](../verification.md).
