---
id: 0012
title: Frame allocator for user address spaces (M5 needs-first)
status: accepted
date: 2026-08-05
accepted: 2026-08-05
---

# ADR-0012: Frame allocator for user address spaces

## Acceptance status

**Accepted** (2026-08-05). This is the **needs-first** design for
[M5](../architecture.md) (EL0 agents with private address spaces). It does
**not** implement a frame allocator; it forbids building one that contradicts
the rules below.

[ADR-0005](0005-static-page-table-arena.md) remains **accepted** for the
**kernel** page-table arena. This ADR is a *companion*, not a full supersede:
kernel map tables stay in the static arena; **user** address-space tables and
user frames come from a separate pool.

## Context

M5 done-when requires a task at EL0 in its own `TTBR0`, a permission fault on
kernel addresses, and `SVC` round-trip. That needs:

- page tables that are **allocated and freed** as address spaces come and go;
- optionally physical frames for user stacks / heaps.

Today:

- Kernel translation tables live in a **fixed `link.ld` arena** (ADR-0005).
- The kernel **heap** is a free-list of virtual ranges already mapped (`GlobalAlloc`).
- There is **no** physical frame pool for ephemeral tables.

Temptations that create debt:

- grow only the 0005 arena until “it fits” — cannot free an AS cleanly;
- pull table pages from the kernel heap — circularity and W^X confusion;
- a full Linux-style buddy “because we will need it” — design without the M5 use case.

## Decision

### 1. Two pools, two roles

| Pool | Role | Lifetime |
| ---- | ---- | -------- |
| **Kernel page-table arena** (ADR-0005) | Identity map / kernel TTBR1 (or equivalent) tables, boot splits | Process lifetime; never used for user AS |
| **Frame allocator** (this ADR) | 4 KiB (or granule) **physical frames** for user AS page tables and M5 user data pages | Alloc on AS create / map; free when AS is torn down |

### 2. Where frames come from

A **reserved physical RAM region** described in layout / BSP (size fixed at
build or boot from a single constant), **disjoint** from:

- kernel image + early BSS/stack;
- kernel heap virtual range;
- the page-table arena bytes in `link.ld`.

Exact base/size are an implementation detail of the M5 PR, but must be
**named** (not “whatever is left after heap”).

### 3. Software shape

- **Pure arithmetic** in `kernel-core` (bitmap or free-list of frame indices),
  host unit-tested, Miri-clean where `unsafe` appears.
- **Kernel** owns phys↔virt for the pool, `alloc_frame` / `free_frame`, and
  only frees frames that the allocator handed out (refuse foreign phys).
- User page tables are filled with `mmu` helpers that take **frames from this
  pool**, never from the kernel arena.

### 4. M5 v1 scope (what the allocator must support first)

Minimum for M5 done-when:

1. Allocate frames for one user page-table tree (enough levels for a small AS).
2. Allocate frames for one user stack (or map a fixed user stack region with
   frames from the pool).
3. Free **all** frames of that AS on teardown (no permanent leak for a
   spawn/exit loop of one agent).

Not required in v1: page cache, COW, swap, huge pages, multi-order buddy.

### 5. Explicit non-goals

- Replacing ADR-0005 for kernel tables.
- Allocating kernel heap backing pages from the frame pool (heap stays as today
  until a separate decision).
- SMP-safe allocator design beyond single-core + IRQ-masked mutation (same class
  as today’s `SyncCell` sched).

## Consequences

### Positive

- M5 can create/destroy AS without lying about free.
- Kernel boot path stays independent of user frame pressure.
- Host tests can nail free-list bugs before EL0 exists.

### Costs

- Another RAM carve-out to size and document.
- Two “out of memory” paths (arena `OutOfTables` vs frame OOM) — both must be
  explicit `Result`s, not silent hangs.

### Gate that would catch a reversal

| Reversal | Signal |
| -------- | ------ |
| User Lx tables from kernel arena | Review / layering; arena remainder collapses under AS create |
| Frames from kernel heap | Circular map/heap; fail multi-role |
| Free without ownership check | Heap-style refuse counter for frames; unit tests |
| Claim M5 done without free on teardown | Spawn/exit leak visible in “frames free” boot/idle line |

## Alternatives considered

| Alternative | Why not |
| ----------- | ------- |
| Only enlarge ADR-0005 arena | No free; wrong lifetime |
| Tables from kernel heap | Circular dependency; rejected in 0005 |
| Full buddy + zone allocator now | Speculative complexity before one EL0 task works |
| Delay any frame pool until M6 | M5 done-when is impossible without it |

## When to revisit

- Multiple concurrent user AS with different sizes (pool sizing).
- Shared libraries / multiple mappings of one frame (refcount).
- M6 device pages: device MMIO is **not** from this pool (see ADR-0013).

## Related

- [ADR-0005](0005-static-page-table-arena.md) — kernel table arena
- [architecture.md](../architecture.md) — M5 done-when
- [ADR-0003](0003-early-mmu.md) — early map before Rust
- Finding F26 / [ADR-0013](0013-narrow-device-windows.md) — device windows (separate)
