---
id: 0083
title: K8 sixth slice — work stealing first code
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0008, 0048, 0075, 0076, 0077, 0080, 0081, 0082]
---

# ADR-0083: K8 sixth slice (work stealing first code)

## Acceptance status

**Accepted** (2026-08-10). Implements the first **code** slice of
[ADR-0082](0082-k8-work-stealing-design.md): hard re-home pull-on-idle steal
of **opt-in** Ready workers. Status **done (QEMU)** and **done (HW)** — Pi
stamp 2026-08-10, transcript `.serial-log/20260810-144305.log` (`smp: steal ok`
+ `cpu: Cortex-A72 r0p3` + `CNTFRQ=54000000`; `hw-transcript-check` clean;
build `src=5c7c4d2c`).

## Decision (what landed)

### 1. Pure model

- `stealeable[slot]` default **false** on admit (first-code opt-in)
- `set_stealeable` / `is_stealeable` / `can_steal_into` / `try_steal_into`
- `try_steal_into`: one pass; rotate non-stealeable heads; re-home; enqueue on thief
- `switch_on`: when current is idle and local queue empty, try steal first
- `RunQueue::front` / `for_each_ready` for probe
- Host unit tests cover re-home, skip pinned, wake on new home, idle yield steal

### 2. Kernel

- `mark_current_stealeable` / `mark_current_not_stealeable`
- Agents (`Agent::create_prepared` / `from_aspace`) force not-stealeable (TLB)
- Secondary idle treats `can_steal_into` as work (timer wakes WFI for pull)
- `MAX_TASKS` 49 → 52

### 3. Evidence

| Line | Meaning |
| --- | --- |
| `smp: steal workers spawned` | Watch + two opt-in victims admitted on CPU0 only |
| `smp: steal ok` | A victim observed `affinity() == 1` without `spawn_on(1)` |
| `smp: steal timeout` | Fail the boot oracle |

No non-yielding holder (would break task-a/b interleave). Two cooperative
victims provide Ready work for the thief.

Gate: `boot-check` / `hw-transcript-check`.

**HW stamp (2026-08-10):** transcript `.serial-log/20260810-144305.log` —
`smp: core1 alive` + `ipi` + `ran` + `preempt-el0-cpu1:*` + `smp: steal ok`
on Cortex-A72; primary K4 oracles also clean on the same boot.

### 4. Residuals (honest)

- Agent / user-AS steal + TLB IPI  
- Soft affinity; push steal; cores 2–3  
- Default product stealeable policy (still opt-in)  

## Related

- Design: [0082](0082-k8-work-stealing-design.md)  
- Queues: [0075](0075-k8-per-core-queues-design.md)/[0076](0076-k8-per-core-queues-first-slice.md)  
