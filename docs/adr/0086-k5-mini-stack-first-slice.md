---
id: 0086
title: K5-S first slice — Mini stacks (one page, no unmapped guard)
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0006, 0023, 0044, 0085]
---

# ADR-0086: Mini stack class (K5-S code)

## Acceptance status

**Accepted** (2026-08-10). Implements **K5-S** from
[ADR-0085](0085-k5-density-residual-design.md): a third stack class that
**halves Thin’s heap cost** for short EL1 workers without raising `MAX_TASKS`
and without collapsing the agent pair.

Status: **done (QEMU)** via `make boot-check`; **done (HW)** Pi stamp 2026-08-10,
transcript `.serial-log/20260810-162926-boot2-k5s.log` (`density: mini n=2
bytes_each=4096`, Cortex-A72, `hw-transcript-check` clean; image `src=afde0d22-dirty`
with Mini code; clean tree `2cffdf8`).

## Context

On a 4 KiB translation granule, “2 KiB usable + unmapped guard” cannot unmap a
half-page. ADR-0085’s first-cut Mini size is therefore realised as:

| Class | Usable mapped | Unmapped guard | Heap per task |
| --- | ---: | --- | ---: |
| Full | 16 KiB | 4 KiB | 20 KiB |
| Thin | 4 KiB | 4 KiB | 8 KiB |
| **Mini** | **4 KiB** | **none** | **4 KiB** |

Mini trades the guard **hole** for density. Overflow is not a translation fault
into a deliberate gap — acceptable only for short yield/exit workers
(`spawn_mini`), not multi-SVC agent drivers.

## Decision

1. `kernel_core::density::StackClass::Mini` + `has_guard_page` / `bytes_per_task`.
2. `TaskStack::allocate_mini()` — one page, no `mmu::unmap`.
3. `sched::spawn_mini` — never silent default for `spawn*`.
4. Oracle: `density: mini n=2 bytes_each=4096` alongside thin (census fixed:
   2 thin + 2 mini, not 3+3, so later oracles still fit — ADR-0085 forbids
   `MAX_TASKS++` as the fix).
5. Non-goals: K5-H multiplex, K5-B pair collapse, Mini as product agent default.

## Gates

| Check | Evidence |
| --- | --- |
| Host pure | `density` unit tests (mini half of thin) |
| QEMU | `density: mini n=[1-9]` in `boot-oracle.sh` |
| HW | same + `hw-transcript-check` |

## Related

- Policy: [0085](0085-k5-density-residual-design.md)
- Thin: [0044](0044-k5-agent-density.md)
