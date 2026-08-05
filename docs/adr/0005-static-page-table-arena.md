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
coverage. User address spaces at M5 use a **separate** frame pool
([ADR-0012](0012-frame-allocator-for-address-spaces.md)); this arena is **not**
replaced for kernel tables.

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

## Amendment — 2026-08-05: the gate above has been written

The decision is unchanged; the text above is left exactly as accepted. This
records that the assertion its gate section calls for — "an assertion in the
boot check on a minimum remainder would close it, and has not been written" —
now exists, so that section should be read as history rather than as the
current state.

`bootstrap::run` refuses to boot when fewer than `MIN_SPARE_TABLES` tables are
free after the kernel map and the DTB mapping, naming the constant to raise.
Exhaustion is no longer something a reader of the boot output has to notice.

What made the reserve necessary rather than merely tidy is M3: unmapping a
task-stack guard inside a 2 MiB block splits that block, taking one table per
split with no path back. The arena therefore stopped being a boot-time cost and
became one that grows with task churn — the shape this ADR anticipated at M5
and met at M3 instead. `mmu::splits()` counts them, and the boot line reports
both numbers.

The reserve is a strengthening, not a change of decision, so it is written here
rather than in a successor. A successor is still what a *different* allocation
strategy would need, and [`ADR-0006`](0006-cooperative-execution-model.md)
already names ADR-0005 as the wrong shape for M5.

## References

`link.ld` (`PAGE_TABLE_ARENA_SIZE`), `src/arch/aarch64/mmu.rs`,
[`mmu.md`](../mmu.md).
