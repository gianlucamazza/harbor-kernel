---
id: 0005
title: Static page-table arena instead of a frame allocator
status: accepted
date: 2026-08-04
accepted: 2026-08-04
---

# ADR-0005: Static page-table arena instead of a frame allocator

## Acceptance

**Accepted 2026-08-04.** The arena is what the running kernel uses; boot prints
`tables_remaining()` and mapping failures surface as `MmuError::OutOfTables` /
boot refusal rather than a silent hang. The weak gate (console remainder, no
automated minimum) remains as declared in this ADR — acceptance does not invent
coverage. Supersede at M5 when address spaces come and go.

## Context

`arch::mmu` allocates translation tables from a 64 KiB arena reserved in
`link.ld` (`PAGE_TABLE_ARENA_SIZE`). Six tables are used by the kernel map; ten
remain, and `tables_remaining()` is printed on every boot.

The temptation, once runtime mapping (`mmu::map`) exists, is to build a frame
allocator straight away. That would be speculative infrastructure: the kernel
maps itself once, plus individual regions the firmware assigns, and never frees a
table.

## Decision

A fixed-size arena, sized at build time, with the remaining space reported at
boot and `MmuError::OutOfTables` as an explicit failure.

This is also what Linux does with `init_pg_dir`: a statically reserved pool for
mapping the kernel, distinct from the frame allocator that address spaces need.

## Consequences

**Positive** — no allocator has to be ready before the first mapping, so there is
no circular dependency between heap and tables; exhaustion is visible before it
becomes a failure (`40960 B of table arena left` on every boot).

**Negative** — it does not support dynamic address spaces. A growing
`MAX_REGIONS`, or a growing number of `mmu::map` callers, must be accompanied by
checking the remainder.

## Alternatives considered

| Alternative                       | Why not                                                                                                                 |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| A frame allocator now             | It is needed when address spaces come and go, i.e. from M5. Building it earlier means designing it without its use case |
| Tables from the heap              | Circular: the heap is mapped by the tables one would allocate from it                                                   |
| A bigger arena, and stop worrying | Moves the limit without making it visible; the printed remainder is what makes the limit observable                     |

## The gate that protects this decision

None. The signal is `tables_remaining()` on the console and
`MmuError::OutOfTables`, which is a `Result` rather than a panic. **This is a
declared weakness** — exhaustion would be noticed by whoever reads the boot
output, not by a check. An assertion in the boot check on a minimum remainder
would close it, and has not been written.

## When to revisit

At M5, or earlier if `mmu::map` acquires many callers. The concrete trigger is
the first address space that is created and destroyed.

## References

`link.ld` (`PAGE_TABLE_ARENA_SIZE`), `src/arch/aarch64/mmu.rs`,
[`mmu.md`](../mmu.md).
