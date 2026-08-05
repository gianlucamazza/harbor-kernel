---
id: 0008
title: IRQ handler policy for cooperative wakes (F13 / M4)
status: accepted
date: 2026-08-04
accepted: 2026-08-05
---

# ADR-0008: IRQ handler policy for cooperative wakes (F13 / M4)

## Acceptance status

**Accepted** (2026-08-05). This is the **needs-first** design for
[M4](../architecture.md) (IPC + cooperative wakes). It closes finding **F13**
as a *decision*: M4 code must not invent a handler shape under the first
mailbox that compiles.

**Code today:** M3 still uses sealed `Handler = fn()` with no cookie. That is
allowed only as the pre-migration state. The **first M4 implementation PR**
must land cookie handlers + a host-tested wake queue matching this ADR; it
must not ship mailboxes that wake by mutating TCBs from IRQ context or by
keying only on raw GIC ids without a cookie registry.

## Context

At acceptance:

- `irq::Handler = fn()` — no cookie, no object, no capability id.
- Dispatch is sealed after bootstrap; registration is bring-up only.
- ADR-0006 forbids **context-switch from IRQ handlers**. IRQs may only touch
  atomics / rings; any wake is posted for the voluntary path.

M4 needs a message or notification to make a **Blocked** task Ready. That wake
will often originate in an IRQ (UART RX today is the template: drain into a
ring, never TX, never switch). Finding **F13** from the 2026-08-04 multi-role
review blocked M4 because `fn()` cannot carry a capability or a wait-queue key.

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

**Negative** — every handler signature changes once at M4 land; cookies need a
registry; wake queue capacity is a new fixed limit.

**Transitional** — until the M4 PR lands, sealed `fn()` handlers remain; they
are not a second accepted design.

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
| Process | Multi-role review before M4 `done (HW)` |
| Code (when implemented) | Host tests on wake queue; layering: `exception` still only `irq`; `sched` not imported from `irq` |
| Mutation | A `sched::yield_now` call from an IRQ handler path must fail review / future grep gate |
| Reversal | M4 mailbox that wakes without cookie/wake-queue shape |

## When to revisit

- First driver-as-agent (M6): mediated register + narrower MMIO (F26).
- SMP: wake queue and cookie registry need real synchronisation.
- If M4 needs multiple waiters per IRQ: extend cookie → wait queue id.

## References

- [ADR-0006](0006-cooperative-execution-model.md) — no IRQ-side switch
- [architecture.md](../architecture.md) — M4 done-when; F13
- `src/irq/mod.rs` — current `Handler = fn()`, `seal` (pre-M4)
- `src/bootstrap/console_loop.rs` — idle as wake consumer template
