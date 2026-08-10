---
id: 0044
title: K5 first slice — thin task stacks for agent density
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0006, 0023]
---

# ADR-0044: Thin stacks for density (K5 entry)

## Acceptance status

**Accepted** (2026-08-08). First slice of **K5**: not every task needs a 16 KiB
kernel stack. Creators choose a **stack class** at spawn so more concurrent
agents fit in the same heap without only raising `MAX_TASKS`.

## Decision

### 1. Pure `kernel_core::density`

| Class | Usable stack | Use |
| --- | --- | --- |
| `Full` | 16 KiB | Default (driver + deep EL0 sessions) |
| `Thin` | 4 KiB | Short EL1 workers / shallow agents |

`usable_bytes(class)`, `bytes_per_task(class)` (usable + guard page), and
`max_tasks_for_heap(heap_bytes, class)` are host-tested pure arithmetic.

### 2. Kernel API

- `sched::spawn_thin(entry)` / `spawn_thin_with_caps` allocate `Thin` stacks.
- Default `spawn*` keep `Full` (no silent shrink of existing demos).

### 3. Oracle

Bootstrap spawns N thin tasks; prints `density: thin n=… bytes_each=…` so capacity
is visible without claiming unlimited scale.

### 4. Non-goals

- Collapsing the EL1 driver half of the agent pair — residual policy
  [ADR-0085](0085-k5-density-residual-design.md) (**K5-H** / **K5-B** deferred;
  first follow-on code is **K5-S** Mini).
- Dynamic stack growth.
- Changing `MAX_TASKS` as the density solution (restated in ADR-0085).

## Gates

| Check | Evidence |
| --- | --- |
| Host pure arithmetic | unit tests |
| QEMU thin spawns | `density: thin n=` |
