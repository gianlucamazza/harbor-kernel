# Post-K8 multi-role review — 2026-08-10

Audit according to [ADR-0001](../adr/0001-multi-role-analysis.md): findings only;
status ownership remains [`roadmap.md`](../roadmap.md). Read-only inventory of
the tree after the K8 arc (unpark → IPI → queues → shared-state → per-core timer
→ EL0-on-CPU1 → steal) and [ADR-0084](../adr/0084-k7-residual-policy.md).

| | |
| --- | --- |
| **Tree** | head after ADR-0084 (`d2bf2ce` family); `sched::MAX_TASKS = 52` |
| **Method** | Fixed roles R1–R12; evidence from docs + source paths; no new code in the audit itself |
| **Previous full pass** | [excellence 2026-08-08](2026-08-08-excellence.md); [doc-comment drift 2026-08-09](2026-08-09-doc-comment-drift.md) |
| **Gap** | Multiple HW stamps landed 2026-08-10 without a multi-role report (ADR-0001 cadence) |

## Executive summary

Harbor is strong on the **gate-shaped path** (ADR → code → QEMU → HW → docs).
Post-K8 debt sits one step outside that path:

1. **Mechanism ≠ product multi-core** — dual-current, preempt, EL0-on-CPU1, and
   steal are **done (HW)**; product agents still home on CPU 0; steal is opt-in
   EL1-only (no agent AS without TLB IPI).
2. **Shape cost (pair, ADR-0023)** — density limited by driver stack + task slot;
   K5 thin stacks paid; **driver-half collapse** remains the H2 mechanism residual.
3. **Oracle census ratchet** — `MAX_TASKS` climbed for concurrent demos (→ 52);
   that is demo tax, not density.
4. **SSOT lag** — architecture/stack/SECURITY still named closed K8 residuals
   and stale bounds (same class as excellence F-1).
5. **Verification asymmetry** — oracle ≫ product image evidence (excellence §2).
6. **Composition product incomplete by policy** — P3/P4 deferred without a
   composition target.

**No P0 safety defect** found in this pass. P1 items are claim drift, density
shape, threat-model lag, and product multi-core policy honesty.

---

## Findings by role

### R1 — Layering

### [P2] R1 — Coarse sched `IrqSpinLock` is correct for N=2 only

- **Aspect:** improvement
- **Evidence:** [ADR-0077](../adr/0077-smp-shared-state-discipline.md); single lock path in `src/sched/`
- **Impact:** Cores 2–3 need a design ADR before unpark
- **Proposed action:** risk-accepted while N=2
- **Effort:** —

### [P1] R1 — Global tables still use `without_irqs` + “single core” SAFETY after EL0-on-CPU1

- **Aspect:** problem (SMP shared-state residual)
- **Evidence:** `src/naming/mod.rs`, `src/storage/mod.rs`, `src/taskcap/mod.rs` mask local IRQs only and document “single core”; EL0-on-CPU1 and steal make concurrent mutators possible if two cores hit resolve/bind/put/get/mint in parallel. Heap and sched already use `IrqSpinLock` (0077).
- **Impact:** Data race / corrupted table under dual-core product load (oracle may not hit it)
- **Proposed action:** extend 0077 discipline — `IrqSpinLock` (or equivalent) for naming/storage/taskcap (and audit console TX); rewrite SAFETY comments
- **Effort:** M
- **Follow-up:** **complete (HW)** 2026-08-10 — first pass: naming, storage, taskcap, ipc, frames, asid, durable, console TX + SCHED/IPC order (`b3d208e`). Completion pass: IRQ wait/caps, console RX line, status, MMU `MAP_LOCK`, drain lock-order fix (`05a3814`). Silicon stamp transcript `20260810-160227.log` (`src=05a38149`, `hw-transcript-check` clean)

### R2 — Memory / MMU

### [P2] R2 — Option C clone cost is map density, not stack density

- **Aspect:** problem (cost), not a bug
- **Evidence:** [ADR-0014](../adr/0014-ttbr-split-m5.md), [ADR-0084](../adr/0084-k7-residual-policy.md)
- **Impact:** TTBR1 only if a named trigger fires
- **Proposed action:** no TTBR1 code; optional K7-M lab
- **Effort:** —

### [P1] R2 — Agent steal without TLB IPI must not be product default

- **Aspect:** problem (policy risk)
- **Evidence:** [ADR-0082](../adr/0082-k8-work-stealing-design.md)/[0083](../adr/0083-k8-work-stealing-first-slice.md); `mark_current_not_stealeable` in `src/agent/mod.rs`
- **Impact:** Silent migration of live AS would break isolation assumptions
- **Proposed action:** keep non-goal; document product pin/home policy
- **Effort:** S (docs)

### R3 — Interrupts / concurrency

### [P1] R3 — Product multi-core incomplete despite mechanism HW

- **Aspect:** problem (product honesty)
- **Evidence:** roadmap/stack “product agents still home CPU 0”; steal opt-in
- **Impact:** Dual-core is lab-proven, not product-default agent placement
- **Proposed action:** explicit product SMP policy in architecture + SECURITY
- **Effort:** S

### [P2] R3 — Global ticks advance only on CPU 0

- **Aspect:** improvement (document)
- **Evidence:** `src/time.rs` CPU0-only tick advance after ADR-0079
- **Impact:** Misreading fairness/timeout symmetry across cores
- **Proposed action:** verification / architecture note
- **Effort:** S

### R4 — unsafe / panic

### [P2] R4 — `CURRENT_EL0` is per-CPU array; SECURITY residual stale

- **Aspect:** problem (doc)
- **Evidence:** `src/arch/aarch64/el0.rs` `CURRENT_EL0: [AtomicPtr; N]`; SECURITY residual “assembly assumes pointer”
- **Impact:** Threat residual list lies about residual surface
- **Proposed action:** re-baseline SECURITY residual
- **Effort:** S

### R5 — Verification

### [P1] R5 — No multi-role report through the K8 HW arc

- **Aspect:** problem (process)
- **Evidence:** last full review 2026-08-08; stamps 2026-08-10 without report
- **Impact:** ADR-0001 cadence missed
- **Proposed action:** this document
- **Effort:** S

### [P1] R5 — Oracle evidence ≫ product image evidence

- **Aspect:** problem
- **Evidence:** excellence §2 product-boot vs default oracle assertion count
- **Impact:** Shipped image less proven than lab image
- **Proposed action:** strengthen product-boot-check (composition minimum)
- **Effort:** M

### [P2] R5 — Mutation testing incomplete on authority modules

- **Aspect:** improvement
- **Evidence:** excellence F-7; `taskcap` / `asid` / `runqueue` historically unmutated
- **Proposed action:** risk-accepted or batch mutants
- **Effort:** M

### R6 — Boot / firmware

### [Risk-accepted] R6 — Firmware Group 0 / blobs / DTB model

- Already owned by ADR-0004 / 0011 / blobs.md

### R7 — Performance / footprint

### [P1] R7 — `MAX_TASKS` ratchet (→ 52) is oracle tax

- **Aspect:** problem
- **Evidence:** `src/sched/mod.rs` `MAX_TASKS = 52`; bootstrap demos per slice
- **Impact:** Density narrative contradicted by raising the ceiling
- **Proposed action:** census audit; avoid further raises without density win; optional oracle feature-split
- **Effort:** M

### [P1] R7 — K5 driver-half residual is the density frontier

- **Aspect:** problem (architecture cost)
- **Evidence:** [ADR-0023](../adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md), [ADR-0044](../adr/0044-k5-agent-density.md); roadmap next = K5 half
- **Impact:** Multi-agent scale without collapsing the pair remains expensive
- **Proposed action:** design ADR before code
- **Effort:** design M / code L

### [P2] R7 — Capacity bounds skew across subsystems

- **Aspect:** improvement
- **Evidence:** `MAX_TASKS=52`, `MAX_TASK_CAPS=32`, `MAX_AGENTS=8`, `MAX_NAMES=8`, irq waiter capacity 32
- **Impact:** Silent under-cap vs scheduler census
- **Proposed action:** capacity model table in architecture
- **Effort:** S

### R8 — Tooling / DX

### [P2] R8 — Gates do not catch semantic SSOT lag

- **Aspect:** improvement
- **Evidence:** excellence F-1; residual phrases like “residuals are steal”
- **Proposed action:** post-stamp checklist + optional greps for known stale phrases
- **Effort:** S

### R9 — Security surface

### [P1] R9 — SECURITY still frames single-core / SMP non-asset

- **Aspect:** problem
- **Evidence:** SECURITY product row “Single-core”; non-assets list includes “SMP, preemption fairness” while both paid on HW
- **Impact:** Threat model understates dual-core TCB and overstates residuals
- **Proposed action:** SECURITY re-baseline post-K8
- **Effort:** M

### [P2] R9 — Creator/loader remains in TCB

- **Aspect:** risk-accepted (H1)
- **Evidence:** SECURITY trust table; ADR-0021
- **Proposed action:** no change until H3 isolation of composition authority

### R10 — Agent roadmap

### [P1] R10 — Finish-OS product services incomplete (P3/P4 policy)

- **Aspect:** problem (product completeness)
- **Evidence:** [ADR-0049](../adr/0049-deferred-residuals.md); roadmap deferred
- **Impact:** “Services as agents” incomplete without composition targets
- **Proposed action:** explicit appliance-without-net/UI horizon or composition targets
- **Effort:** S (policy) / L (impl)

### [P1] R10 — Multi-agent density blocked on K5 half

- Same as F-R7-2; mission-fit next mechanism

### [P2] R10 — Store `MAX_AGENTS=8` ≪ scheduler tasks

- **Aspect:** improvement
- **Evidence:** `kernel_core::agentstore::MAX_AGENTS`
- **Proposed action:** document; raise only with product need

### R11 — Documentation

### [P1] R11 — architecture.md still lists steal / CPU1 preempt / EL0-on-CPU1 as residuals

- **Evidence:** architecture § How Harbor differs (~lines 69–73 at audit)
- **Proposed action:** fix (Tier A)

### [P1] R11 — stack.md “CPU1 runs idle + proof marker” obsolete

- **Evidence:** stack runtime table
- **Proposed action:** fix (Tier A)

### [P1] R11 — SECURITY `MAX_TASKS` **40** vs code **52**

- **Proposed action:** fix (Tier A)

### R12 — kernel-core API

### [P2] R12 — No composition-facing affinity/home field

- **Aspect:** improvement
- **Evidence:** steal opt-in pure; loader has no `home_cpu` in manifest
- **Proposed action:** optional small design if product pins agents
- **Effort:** M

---

## Optimisations (falsifiable only)

| ID | Hypothesis | Falsify how | When |
| --- | --- | --- | --- |
| O1 | Driver-half collapse ≥2× density vs thin-only | K5 design + census | Next mechanism |
| O2 | Pinning half product agents on CPU1 improves throughput without agent steal | Dual-core composition load | After policy doc |
| O3 | Oracle feature-split lowers required `MAX_TASKS` | Product vs full-oracle census | Hygiene |
| O4 | Switch-cost too low to justify TTBR1 | K7-M on Pi | Only if map density burns |
| O5 | Coarse lock not bottleneck at N=2 | Cycle counts around switch | Before cores 2–3 |

---

## Closed / not problems

- K4 EL0+EL1 preemption **done (HW)**
- K7 ASID first **done (HW)** + residual policy ADR-0084
- K8 through steal **done (HW)** (agent+TLB residual explicit)
- Option C is deliberate regime, not a temporary hack
- Layering import gate operational

---

## Recommended backlog (mission fit)

| Tier | Work |
| --- | --- |
| **A** | SSOT hygiene + this review + capacity model + product SMP policy note + SECURITY re-baseline |
| **B** | Design ADR K5 driver-half → code later |
| **C** | Product multi-core policy enforcement (manifest home) + stronger product-boot-check |
| **D** | Trigger-only: K7-M, TTBR1, agent+TLB steal, P3/P4, H3 L1+, cores 2–3 |

Order: **A → B**. Do not implement TTBR1 or P3/P4 without triggers/targets.

---

## Follow-up of this report

Tier A fixes that land with or immediately after this report are **responses**
to findings, not part of the audit method. Status flips remain owned by
`roadmap.md` with evidence.

| Finding / backlog | Response |
| --- | --- |
| F-R5-2 / Tier **C** stronger product-boot-check | **Paid:** composition-minimum `qemu-product-boot-check.sh` (~35 layered asserts on the shipped path) + `make oracle-census` in `make check` (MAX_TASKS source ↔ architecture table ↔ documented raise). Not a second full oracle. |
| Tier **C** product multi-core policy enforcement | **Open** (manifest home / pin) — separate from the boot-gate hygiene slice. |
| Tier **A** SECURITY re-baseline / remaining SSOT | **Open** as standing hygiene; capacity model already in `architecture.md`. |
