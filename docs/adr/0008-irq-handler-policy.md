---
id: 0008
title: IRQ handler policy for cooperative wakes (F13 / M4)
status: accepted
date: 2026-08-04
accepted: 2026-08-05
---

# ADR-0008: IRQ handler policy for cooperative wakes (F13 / M4)

## Acceptance status

**Accepted** (2026-08-05). Closes finding **F13**. M4 is **done (HW)** with
this shape in tree: cookie handlers, host-tested `WakeQueue`,
`poll_wakes` on the voluntary path, mailboxes that do **not** switch from IRQ.

**Code today (post-M4):**

- `irq::Handler = fn(IrqCookie)` with cookie stored at `register` (`src/irq`).
- Timer / UART handlers ignore the cookie but take the signature.
- `kernel_core::wake::WakeQueue` + `sched::wake_from_irq` / `poll_wakes`.
- IPC send may `wake_task` on the voluntary path only.

Reversal = inventing a second handler type, switching from IRQ, or waking by
raw GIC id without the wake queue / cookie registry.

## Context

When this ADR was written, handlers were still bare `fn()` sealed at boot.
ADR-0006 forbids context-switch from IRQ handlers. M4 needed a way to make a
**Blocked** task Ready from IRQ without violating that rule. Finding **F13**
blocked M4 until this shape was recorded and then implemented.

## Decision

### Handler shape (M4 minimum)

One signature for all registered lines:

```text
type IrqCookie = u32;           // index or generation-tagged id; not a pointer
type Handler = fn(IrqCookie);
```

- Cookies are assigned at **registration** (still before `seal` in M4, or via a
  mediated register path later). They are not forgeable from EL0 until
  capabilities exist; until then they are kernel-internal indices.
- The cookie is what a future capability **names** (an IRQ notification object),
  not a raw GIC id handed to agents.
- Lines that need no wake (e.g. pure accounting) still take a cookie; the
  handler may ignore it. No second `fn()` type — dual shapes invite drift.

### Wake policy (with ADR-0006)

On interrupt:

1. Claim / handle device (unchanged layering).
2. May **push** a wake record (task id, or “event bit”) into a lock-free
   structure owned by the scheduler — same class as today’s RX ring.
3. Must **not** call `sched::yield_now`, `context_switch`, or mutate TCB
   stacks.
4. The running task (usually idle) observes wakes on the voluntary path
   (`has_ready` / a `poll_wakes` step in idle) and enqueues Blocked→Ready.

### Softirq vs direct

M4 uses a **single-producer (IRQ) / single-consumer (scheduler) wake queue** in
`kernel-core` (host-tested), not a full Linux softirq hierarchy. If capacity is
exhausted, count drops (same honesty as RX ring drops) rather than spinning in
IRQ.

### Sealing

`irq::seal` remains for the static timer + UART lines. Dynamic registration for
driver agents is **M6** and needs a successor ADR (capability-mediated
`register`). This ADR does not reopen seal for arbitrary runtime handlers.

### What this ADR does not decide

| Concern | Where |
| --- | --- |
| Mailbox ABI / message layout | M4 design / successor |
| Capability bit format | later |
| Preemption on IRQ | forbidden by ADR-0006 unless superseded |
| EL0 delivery of IRQs | M5/M6 |

## Consequences

**Positive** — F13 has a recorded shape before mailbox code; IRQ/sched stay
separated; pure wake-queue arithmetic is testable; caps can later wrap the same
cookie.

**Negative** — handler signature is fixed to cookies forever; wake queue
capacity is a fixed limit (drops under pressure, like the RX ring).

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Keep `fn()` and use global tables keyed by IRQ id only | No room for cap_irq; forces one waiter per line |
| Dual `fn()` / `fn(IrqCookie)` forever | Two paths drift; cookie is free to ignore |
| Switch to the waiter from the IRQ handler | Violates ADR-0006; races multi-claim IRQ entry |
| Full async executor in IRQ | Hides stacks; fails M3/M4 overflow story |
| Dynamic `register` without seal for M4 | Reopens RMW/race surface; not required for two demos |

## The gate that protects this decision

| Layer | What it catches |
| --- | --- |
| Process | Multi-role before later IRQ-to-agent work (M6) |
| Code | Host tests on `WakeQueue`; layering: `exception` → only `irq`; `sched` not imported from `irq` |
| Mutation | `sched::yield_now` / `context_switch` from an IRQ path — review / grep |
| Reversal | Bare `fn()` handlers again, or TCB mutation from IRQ |

## When to revisit

- First driver-as-agent (M6): mediated register + narrower MMIO (F26).
- SMP: wake queue and cookie registry need real synchronisation.
- If M4 needs multiple waiters per IRQ: extend cookie → wait queue id.

## References

- [ADR-0006](0006-cooperative-execution-model.md) — no IRQ-side switch
- [architecture.md](../architecture.md) — M4 done (HW); F13 closed
- `src/irq/mod.rs` — `Handler = fn(IrqCookie)`, `seal`
- `crates/kernel-core/src/wake.rs` — SPSC wake queue
- `src/sched/mod.rs` — `wake_from_irq` / `poll_wakes`
- `src/bootstrap/console_loop.rs` — idle drains wakes
