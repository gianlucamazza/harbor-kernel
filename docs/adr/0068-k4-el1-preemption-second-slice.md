---
id: 0068
title: K4 second code slice — same-EL (EL1) IRQ preemption
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0006, 0008, 0022, 0046, 0051, 0058, 0062, 0064]
---

# ADR-0068: EL1 IRQ preemption (K4 second slice)

## Acceptance status

**Accepted** (2026-08-09), on delegated authority (same standing delegation
as ADR-0064/0066; session directive "pianifica, correggi e completa").

Implements the remainder of the [ADR-0051](0051-k4-irq-preemption-design.md)
design and **partially supersedes
[ADR-0006](0006-cooperative-execution-model.md)** — see the supersession
table below.

## Context

ADR-0064 rotates a spinning **EL0** session at the lower-EL IRQ epilogue.
An **EL1** task that never yields still owns the CPU until a voluntary
checkpoint. This slice evaluates the same monotone predicate
(`kernel_core::preempt::should_set`, unchanged — second call site, no new
model) on the **same-EL** IRQ-return epilogue, after claim → dispatch → EOI.

## Decision: frame-on-own-stack pivot, no new resume mode

The EL1t IRQ vector epilogue asks `sched::el1_preempt_pending()` (pure
atomics + tick read; `STARTED` gates the boot window, `CURRENT_IS_IDLE`
keeps idle out). When true, `el1_preempt_pivot` (asm, `vectors.s`) runs:

1. While still on SP_EL1: `mrs x1, sp_el0` — the interrupted task's stack
   pointer, intact because the kernel runs SPSel=0 and the exception
   hardware switched to SP_EL1. Reserve `TRAP_FRAME_SIZE` (272 B) below it
   and copy the trap frame there (17 `ldp`/`stp` pairs; the `.equ` ties the
   size to the Rust struct).
2. `msr sp_el0`, unwind SP_EL1, `msr spsel, #0` — the frame now lives on
   the preempted task's own stack and the shared exception stack is empty
   **before** any switch. No live data ever sits above SP_EL1's top; SP_EL0
   is directly readable/writable from EL1 while SPSel=1, so there is no
   uncovered window.
3. `bl el1_preempt_from_irq` (Rust, in `sched`): an ordinary
   `switch_with(Switch::Preempt)` on the task's own stack. The preempted
   task's saved continuation is simply the pivot's tail: on resume it
   restores the frame from its own `sp` (`kernel_exit` shape) and `eret`s.
   No per-TCB save area, no new scheduler mode, no flag.

DAIF is fully masked from exception entry to the `eret`; the SPSR restore
reopens the interrupted task's I bit. Softfloat kernel (ADR-0002): no
FP/SIMD state exists to save. The `bl` from asm to `sched` symbols is a
deliberate seam (like `CURRENT_EL0`): no Rust import edge, layering gates
untouched.

**Central invariant:** preemption fires only where PSTATE.I was open, so
every `cpu::without_irqs` / `irq_save` region is, by construction, a
non-preemptible critical section — ADR-0022's mask discipline becomes the
kernel's preemption-safety contract.

## ADR-0006 supersession table

| ADR-0006 rule                  | Now                                                                                                        |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| No IRQ-side context switch     | **Superseded**: the EL1t vector **epilogue** (after EOI) may rotate the current task                       |
| IRQ handlers only post work    | **Stays**: `handle_cpu_irq` and every device handler still never switch (ADR-0008); only the epilogue does |
| Voluntary yield / park primary | **Stays**: preemption is the quantum backstop                                                              |
| Idle WFI model                 | **Stays**: idle is unpreemptable (`CURRENT_IS_IDLE`, model `Stay`); its WFI sits inside a mask anyway      |
| EL1h fault-in-fault fatal      | **Stays**                                                                                                  |

## Re-audit (closes ADR-0051's "must be re-audited" clause)

1. **Wake-queue SPSC drain** — the one real hazard found: `irq::wait::drain`
   popped the single-consumer queue unmasked; a preemption mid-drain would
   schedule a second concurrent consumer. **Fixed in this slice**: the
   whole drain loop is one masked region (bound = queue capacity, O(1)
   callbacks, nested masks fine). `el1_preempt_from_irq` calls **no**
   `poll_wakes` — minimal IRQ-context work, and never a drain from the
   epilogue path; idle and `yield_now` still drain, so wake latency is
   unchanged. `poll_park_timeouts` needs nothing: `table.poll` consumes
   expired entries under one mask and the unmasked half works on a private,
   epoch-checked (ADR-0062) snapshot.
2. **ADR-0046 budget demo** — structurally raced: the tick that made
   `budget_expired()` true now fires the epilogue first, and `SLICE_START`
   resets on switch-in, so the cooperative workers' observation window
   never opens and `budget: rotated` could never print again. The pair is
   **repurposed** (same spawn slots): a spinner that never yields plus a
   voluntary peer that counts heartbeat-advancing rounds — oracle
   `preempt-el1: rotated`, a strictly stronger claim (rotation without
   cooperation). ADR-0046 is reconciled; `sched::budget_expired` is removed
   with it (no caller remains; the quantum arithmetic lives on in
   `kernel_core::budget` under both preemption paths).
3. **`sched::transfer_held_to_peer` four masked regions** (re-opened by
   ADR-0064): the gaps carry only _names_ (CapId, TaskId, slot indices),
   never contents. `from_slot`'s content is first read inside
   `transfer_held`'s single final masked region, where target liveness (the
   ADR-0062 epoch) and the ADR-0055 band filter judge the _current_ content
   atomically — a preemption in a gap linearizes the operation there, which
   slot-indexed authority (ADR-0017) declares correct. No generation
   counter needed.
4. **`switch(Exit)` → `taskcap::revoke_task` window** (ADR-0057): the whole
   region runs under `switch_with`'s own mask; preemption is DAIF-gated —
   unreachable, now and structurally.
5. **`SyncCell` sweep**: every user is masked, sealed (`irq` dispatch
   `STATE`: no writer after `seal`, IRQ-context reads race nothing), or
   boot-gated — the loader's unmasked `NAME_POOL`/`STORE_ENTRIES` window is
   now _mechanically_ preemption-free (`STARTED == 0` gates the predicate);
   its SAFETY comments say so. `mm` ARENA is covered by its callers' masks
   (every `map`/`unmap` runs under `without_irqs`).
6. **Console `with_tx`**: fully masked closures — correct, and a long
   `kprintln` merely defers the preemption to the first unmasked instant
   (the predicate is monotone). Accepted bounded latency, not a defect.
7. **`irq::wait` capacity comment**: pre-existing drift ("capacity ≥ task
   count", false since MAX_TASKS grew to 42 over Q=32) corrected — drops
   are counted and waiters re-check pending around the park.

## Evidence

| Check                                                            | Gate                                                                                    |
| ---------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Non-yielding **EL1** spinner loses the CPU; peer proves rotation | QEMU `preempt-el1: rotated` + `preempt-el1: spinner exited` (boot-oracle, both runners) |
| EL0 slice unaffected                                             | `preempt: rotated` (ADR-0064) green in the same boot                                    |
| Whole oracle set green in one boot                               | boot-check / hw-transcript-check                                                        |
| No switch under a mask                                           | `irq-scope` (`el1_preempt_from_irq` in SWITCHERS)                                       |
| Rotations visible                                                | `invariants: … preempts=` counts both slices (shared counter)                           |
| **HW stamp (2026-08-09)**                                        | transcript `.serial-log/20260809-151021.log`: `preempt-el1: rotated` + `spinner exited` + `cpu: Cortex-A72 r0p3` + `CNTFRQ=54000000`; `hw-transcript-check` clean |

## Acceptance status (evidence)

**Done (HW)** on Pi 4B stamp 2026-08-09 (transcript above). Code and QEMU
evidence landed with acceptance; silicon stamp closes the K4 residual.

## Residuals / non-goals

Priority scheduling; per-agent quanta; K8 SMP IPI preemption (the
`switch(Exit)`→revoke window re-opens **there**, as ADR-0057 already
records); preemption inside device handlers; softirq/deferred work; FIQ;
EL1h. Mutation-freshness note (ADR-0058): `kernel-core` is untouched by
this slice (the predicate is reused), so new mutation exposure is limited
to the demo and the asm/glue, which the boot oracle covers.
