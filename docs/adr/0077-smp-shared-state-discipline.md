---
id: 0077
title: SMP shared-state discipline — IrqSpinLock, per-CPU mirrors, honest dual-current
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0008, 0048, 0074, 0075, 0076]
---

# ADR-0077: SMP shared-state discipline

## Acceptance status

**Accepted** (2026-08-10). Retires the ADR-0076 workarounds (immortal marker,
quiet-park secondary, dual resched flags, global quantum clobber shields) with
the discipline those workarounds were hiding. Queues evidence: **done (QEMU)**
and **done (HW)** with [ADR-0076](0076-k8-per-core-queues-first-slice.md)
(stamp 2026-08-10, transcript `20260810-130305.log`).

## Context

ADR-0076 proved dual-current but papered over missing mutual exclusion on the
heap and incomplete per-CPU run state with temporary product behaviour. That is
**debt**, not progressive incompleteness.

## Decision

### 1. Critical section shape

Every SMP-shared mutable structure is entered as:

1. mask local IRQs  
2. acquire spinlock  
3. mutate  
4. release spinlock  
5. restore IRQs  

Implemented as `sync::IrqSpinLock`. Used by the kernel heap and the scheduler
(and park-deadline table). Device/SGI handlers never take these locks.

### 2. Heap is multi-core-safe

`with_heap` uses `HEAP_LOCK.with(...)`. A task on CPU 1 may `exit` and free its
stack while CPU 0 allocates. The boot heap oracle treats concurrent free as
growth of free_bytes, not a leak (`after < before` is the only LEAKED case).

### 3. Single resched owner

`arch::smp::{request_resched, take_resched}` own the per-CPU bits. `sched`
forwards; board SGI handlers call `smp` only (layering). No second flag set.

### 4. Per-CPU quantum mirrors

`SLICE_START[N]` and `CURRENT_IS_IDLE[N]` are indexed by affinity. CPU 1 has
no timer PPI yet — `el1_preempt_pending` returns 0 for `affinity != 0` because
there is **no quantum path**, not because queues are incomplete.

### 5. Marker and secondary are ordinary

- CPU1 marker sets `CORE1_RAN` and **exits**; stack is collected.  
- Secondary idle loop is permanent (`poll_wakes` → yield / WFI), no quiet-park.  
- EL0 publish remains CPU0-only: no product agent home on CPU 1 (honest
  non-goal until an EL0-on-secondary ADR).

### 6. Residuals (honest only)

- Work stealing  
- Per-core timer / K4 preemption on core 1 — **code** [ADR-0079](0079-k8-per-core-timer-preemption-first-slice.md) (design [0078](0078-k8-per-core-timer-preemption-design.md)); HW residual
- EL0 agents with home on CPU 1  
- Lock refinement if measured contention requires it  

## Related

- [0075](0075-k8-per-core-queues-design.md), [0076](0076-k8-per-core-queues-first-slice.md)
- [0008](0008-irq-handler-policy.md)
