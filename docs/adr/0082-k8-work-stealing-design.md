---
id: 0082
title: K8 design — work stealing between per-core ready queues
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0006, 0008, 0048, 0050, 0070, 0074, 0075, 0076, 0077, 0078, 0079, 0080, 0081]
---

# ADR-0082: Work stealing (design)

## Acceptance status

**Accepted as design** (2026-08-10). After dual-current sticky queues, fair
quantum on both CPUs, and EL0-on-CPU1 (**done (HW)** —
[ADR-0075](0075-k8-per-core-queues-design.md)–[0081](0081-k8-el0-on-cpu1-first-slice.md)),
the last structural multi-core residual named since
[ADR-0048](0048-k8-smp-design.md) is **load balance without explicit pin**.
This ADR is the design. First **code**: [ADR-0083](0083-k8-work-stealing-first-slice.md)
(**done (QEMU)**; HW stamp residual).

## Context

| Piece | Today |
| --- | --- |
| `queue[cpu]` + `current[cpu]` + sticky `home[id]` | **done (HW)** |
| Wake → enqueue on `home` + SGI resched | **done (HW)** |
| EL1 + EL0 quantum on CPU 1 | **done (HW)** |
| Default product spawn | Still **home = 0** unless `spawn_on` |
| Migration / steal | **None** — idle CPU1 cannot run work that only exists on CPU0’s ready list |
| TLB shootdown IPI | **Absent** — 0080 deferred cross-core AS migration to a steal ADR |

Without steal, dual-core is **fair for pinned work** but not **self-balancing**
when creators keep defaulting to CPU 0. Explicit `spawn_on(1)` remains valid
policy; steal is the mechanism that makes unpinned overload use the second core.

## Decision

### 1. Hard re-home (not soft affinity)

| Item | Choice |
| --- | --- |
| On successful steal | Update `home[id] = thief` permanently (until a later steal moves it again) |
| Wake after steal | Unchanged rule: enqueue on `home` — now the thief |
| Soft affinity / preferred home + run-cpu | **Out of scope** for this design and first code |

**Rationale:** one identity for “where Ready lives” matches ADR-0075’s
mental model and avoids dual paths in every wake producer. Soft affinity is a
later refinement if measured thrashing demands it.

### 2. Pull-on-idle (thief-driven), not push

```text
// Called only under the sched lock, IRQs masked on the thief, never from
// an SGI handler (ADR-0075 §4 / ADR-0008).

try_steal_into(thief):
  if has_ready_on(thief): return false          // local work first
  peer = 1 - thief                              // N_CPUS == 2
  if !has_ready_on(peer): return false
  id = dequeue one Ready from queue[peer]       // never Running, never idle
  if id not stealeable: re-enqueue or refuse; return false
  home[id] = thief
  enqueue id on queue[thief]
  return true
```

**When to call** (code ADR names the exact call sites; design requires all of):

| Site | Rule |
| --- | --- |
| `switch_on(thief, …)` before choosing next | If local ready empty after accounting for the leaving task |
| Idle / secondary loop about to WFI | If local ready empty (same as today `has_ready_on` check, extended) |

**Not** from: SGI handler, device IRQ handler, timer handler body.

No extra IPI for pull: the thief is already running the schedule path. Push
steal (overloaded core kicking a peer) is a residual.

### 3. Who is stealeable (first code era)

| Class | First code | Why |
| --- | --- | --- |
| Idle identities | **Never** | Per-CPU; not on ready queues as workers |
| Task **Running** on peer | **Never** | Would double-run; only Ready on peer’s queue |
| EL1 worker, no live user AS / not mid-EL0 | **Yes** | No TLB cross-core requirement |
| Agent / task with **live EL0 session** (entered EL0, session not ended) | **No** | Needs TLB IPI / ASID policy — second slice |
| Task that **owns a user address space** but is between sessions (session ended, AS still held) | **No** in first code | Conservative: AS may still have nG TLB residue on peer |
| Console server / tasks that must stay on CPU 0 by product rule | **No** if tagged; first code may simply never steal tasks that hold the console-server role role — or never pin them as stealeable | Policy; code ADR may use a simple “not stealeable” bit or role check |

**First-code stealeable = opt-in flag** (`stealeable` default **false** on
admit). Oracle victims call `mark_current_stealeable`; agents force clear on
AS create. This avoids migrating console printers (task-a/b interleave) and
any user-AS task without a TLB story.

The pure model owns `try_steal_into` + the flag; the kernel sets the flag.
This design forbids claiming agent migration without a TLB story.

### 4. Locks and IPI (unchanged doctrine)

| Rule | Source |
| --- | --- |
| Steal mutates shared `Tasks` under the existing sched `IrqSpinLock` | ADR-0077 |
| Local IRQs masked while holding the lock | ADR-0075 |
| SGI handler only sets resched flag — never steal, never switch | ADR-0008 / 0075 |
| After steal, no mandatory IPI (thief schedules locally) | This ADR |

Coarse lock remains acceptable for N=2; lock refinement is still a residual
if measured.

### 5. Thrash and fairness

| Rule | First code |
| --- | --- |
| At most **one** task stolen per steal attempt | Yes |
| Steal only when thief’s ready list is empty | Yes |
| Steal of a task that was just stolen the other way within N ticks | Optional; not required for first oracle |
| Quantum / preempt | Unchanged — stolen task gets a normal slice on the thief |

### 6. Pure model (`kernel_core::tasks`)

Host-tested additions (names free in code ADR):

- `try_steal_into(thief: u8) -> bool` implementing §2 under the model’s
  existing invariants (epochs, states, idle refusal)
- Host tests: two Ready on CPU0, CPU1 empty → steal → one Ready on CPU1 with
  `home == 1`; wake after steal enqueues on CPU1; never steal idle; never
  steal Running

`switch_on` may call `try_steal_into` when it would otherwise go idle with peer
ready — or the kernel may call it immediately before `switch_on` when
`!has_ready_on(self)`. Either is fine if the decision is pure and tested.

### 7. Evidence (for the code ADR)

| Claim | Gate (sketch) |
| --- | --- |
| Steal observed | Oracle e.g. `smp: steal ok` — worker admitted only on CPU0 later runs on affinity 1 (flag set on CPU1, printed on CPU0) |
| No pin cheat | Oracle workers must **not** use `spawn_on(1, …)` |
| Sticky pin still works | Existing `spawn_on(1)` oracles (marker optional; 0079/0081 pairs) stay green |
| No regression | Full `boot-check` / later HW stamp |
| Pure model | Host unit tests for §6 |

Prefer one strong structural claim (flag from affinity 1 without pin) over
timing-sensitive “wait N ms”.

### 8. First implementation slice (follow-on code ~0083)

Ordered:

1. Pure `try_steal_into` + host tests  
2. Stealeable filter in kernel (EL1 plain workers only)  
3. Call from schedule path when local ready empty  
4. Oracle: overload CPU0 only → CPU1 runs a stealer victim  
5. Docs: **done (QEMU)** then HW stamp  

### 9. Explicit non-goals (this design / first code)

- Soft affinity / preferred-home  
- Push steal / work-sharing queues  
- Cores 2–3  
- Stealing **Running** tasks  
- Stealing mid-EL0 or agent AS without TLB IPI  
- New SGI for TLB shootdown (second slice if needed)  
- Lock-free remote dequeue  
- Changing default product spawn to round-robin (policy, not steal)  
- Console TX from core 1  

### 10. Second slice (named residual, not this code)

| Item | Trigger |
| --- | --- |
| Steal agent / user AS | Product needs automatic balance of EL0 drivers |
| TLB IPI or equivalent ASID rollover | Required before agent steal |
| Push / proactive balance | Idle pull insufficient under measurement |

## Alternatives considered

| Alternative | Why not first |
| --- | --- |
| Soft affinity only | Dual state for wake/home; harder audit than hard re-home |
| Steal any Ready including agents | Silent TLB incorrectness without IPI |
| Global queue again | Rewrites 0075; worse cache locality |
| No steal — only `spawn_on` policy | Valid product choice, but leaves 0048 residual open; creators must pin |
| Steal from IRQ path | Violates handler doctrine |

## Consequences

### Positive

- Unpinned load can use both cores  
- Completes the ADR-0048 “queues + steal later” arc at the design layer  
- Keeps TLB complexity out of the first code by forbidding agent steal  

### Residual after first code

- Agent/AS steal + TLB IPI  
- Soft affinity if thrash measured  
- Push steal; cores 2–3; finer locks  

### Gates (design-level)

| Reversal | Catch |
| --- | --- |
| Steal mid-EL0 without TLB story | Review fail; missing non-goal |
| Steal from SGI handler | Deadlock / 0077 violation; irq-scope |
| Wake still uses old home after migrate | Stuck Ready on wrong core; oracle never fires |
| Oracle uses `spawn_on(1)` | False positive “steal” |

## Related

- Parent SMP: [0048](0048-k8-smp-design.md)  
- Queues / sticky: [0075](0075-k8-per-core-queues-design.md), [0076](0076-k8-per-core-queues-first-slice.md), [0077](0077-smp-shared-state-discipline.md)  
- Preempt / EL0-on-CPU1: [0078](0078-k8-per-core-timer-preemption-design.md)–[0081](0081-k8-el0-on-cpu1-first-slice.md)  
- IRQ doctrine: [0008](0008-irq-handler-policy.md)  
- ASID: [0050](0050-k7-asid-first-slice.md)  
