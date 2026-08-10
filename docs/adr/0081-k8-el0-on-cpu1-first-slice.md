---
id: 0081
title: K8 fifth slice — EL0 sessions and preemption on CPU 1
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0016, 0017, 0019, 0023, 0048, 0064, 0070, 0075, 0076, 0077, 0078, 0079, 0080]
---

# ADR-0081: K8 fifth slice (EL0 on CPU 1 first code)

## Acceptance status

**Accepted** (2026-08-10). Implements the first **code** slice of
[ADR-0080](0080-k8-el0-on-cpu1-design.md): per-CPU published EL0 sessions,
publish on every affinity at switch, EL0 quantum preemption on home=1.
Status **done (QEMU)** and **done (HW)** — Pi stamp 2026-08-10, transcript
`.serial-log/20260810-134826.log` (`preempt-el0-cpu1: rotated` +
`spinner exited` + `cpu: Cortex-A72 r0p3` + `CNTFRQ=54000000`;
`hw-transcript-check` clean; build `src=b898ebcd`).

## Decision (what landed)

### 1. Per-CPU `CURRENT_EL0`

- `CURRENT_EL0: [AtomicPtr<El0Session>; 2]` — symbol is the array base
- `publish` / `published` / `current` index by `cpu::affinity()`
- Asm (`load_session`, `restore_kernel_ttbr0_*` in `vectors.s`) loads
  `CURRENT_EL0[MPIDR.Aff0]` (clamp ≥2 → 0)

### 2. Publish on every switch

`switch_with` always calls `publish_el0` (removed the `cpu == 0` fence).

### 3. Spawn

Existing `spawn_on(1, entry)` already allocates `El0Session` and sticky home.
No new API required for the first oracle pair.

### 4. Evidence

| Line | Meaning |
| --- | --- |
| `preempt-el0-cpu1: workers spawned` | Watcher + peer + spinner admitted |
| `preempt-el0-cpu1: rotated` | Non-yielding EL0 spinner on home=1 lost the CPU |
| `preempt-el0-cpu1: spinner exited` | Stop word ended the session |
| peer gave up / watch timeout | Fail the boot oracle |

Workers on CPU 1 use atomics only (no console TX). Watcher on CPU 0 prints.

Primary K4/K8 oracles unchanged. `MAX_TASKS` 46 → 49.

Gate: `boot-check` / `hw-transcript-check`.

**HW stamp (2026-08-10):** transcript `.serial-log/20260810-134826.log` —
`smp: core1 alive` + `ipi` + `ran` + `preempt-el1-cpu1:*` +
`preempt-el0-cpu1: rotated` + `spinner exited` on Cortex-A72; primary K4
oracles also clean on the same boot.

### 5. Residuals (honest)

- Work stealing — **done (HW)** [ADR-0083](0083-k8-work-stealing-first-slice.md)
- Cores 2–3  
- Product default home still 0 (explicit pin only)  
- TLB IPI if steal lands later  
- Console TX from core 1  

## Related

- Design: [0080](0080-k8-el0-on-cpu1-design.md)  
- EL0 preempt: [0064](0064-k4-el0-preemption-first-slice.md)  
- CPU1 EL1 preempt: [0079](0079-k8-per-core-timer-preemption-first-slice.md)  
