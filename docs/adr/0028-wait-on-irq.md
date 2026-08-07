---
id: 0028
title: K1 first slice — wait on IRQ cookie (EL1)
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0006, 0008, 0022, 0023]
---

# ADR-0028: Wait-on-IRQ (K1 entry)

## Acceptance status

**Accepted** (2026-08-07). First slice of completeness track **K1**: an EL1
task can park until a registered IRQ line signals its cookie, without polling
device registers as the only path.

## Context

[ADR-0008](0008-irq-handler-policy.md) shipped cookie handlers and an SPSC wake
queue, but **no producer** called `wake_from_irq`. Timer and UART handlers only
account and drain rings. Device agents that need "sleep until the line fires"
still spin on MMIO (e.g. PL011 `DR` poll).

[ADR-0006](0006-cooperative-execution-model.md) forbids switching from IRQ.
Wakes must post a token and let the voluntary path make the waiter Ready.

Layering forbids `irq` → `sched` and historically forbade `time` → `sched`, so
handlers could not call `wake_from_irq` without a port both sides may use.

## Decision

### 1. IRQ wait port (`irq::wait`)

- One **armed waiter** at a time for v1: `(cookie, task_token)`.
- `arm(cookie, task)` — voluntary path, before park.
- `signal(cookie)` — IRQ path only: if the armed cookie matches, set a
  delivered flag, push `task` onto an SPSC queue owned by `irq`, disarm.
- `drain` — voluntary path (`sched::poll_wakes`) pops tokens and
  `wake_task`s them.
- `take_delivered` — closes the lost-wakeup window between arm and block.

Handlers call **`irq::signal(cookie)` only** (no `sched` import). Timer and
UART already receive their cookies at registration (1 = timer, 2 = UART).

### 2. `sched::wait_for_irq(cookie)`

EL1 API (bootstrap / driver tasks, not yet an EL0 syscall):

1. `arm(cookie, current)`
2. If `take_delivered()` → return (IRQ already posted)
3. `block_current()`
4. Clear delivered on resume

Never call from IRQ or idle.

### 3. Layering

| Module | May import (delta) |
| --- | --- |
| `time` | `irq` (for `signal` only) |
| `console` | `irq` (for `signal` only) |
| `sched` | `irq` (for arm / drain / delivered) |

`irq` still does not import `sched`.

### 4. Non-goals of this ADR

- EL0 syscall / IRQ capability in an agent slot (successor).
- Multiple waiters per cookie.
- Dynamic `register` after seal.
- Replacing UART RX ring drain with wait-only (kernel idle still owns RX).

## Consequences

### Positive

- First real producer for the ADR-0008 wake path.
- Timer wait is host-visible via oracle line `irq-wait: woke`.
- Device-agent sleep can build on the same port later.

### Negative / debt

- Single waiter; a second concurrent wait is refused or overwrites (v1: last arm wins; document and count).
- EL0 still cannot name an IRQ wait without a later ABI ADR.

### Gates

| Reversal | Gate |
| --- | --- |
| No producer again | `make boot-check` asserts `irq-wait: woke` |
| Layering edge | `make layering` |
| Lost wakeup / hang | QEMU boot-check timeout fails the gate |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Poll `time::ticks` in a yield loop | Not first-class IRQ wait; busy-cooperative |
| Switch from timer handler | Forbidden by ADR-0006 |
| Full IRQ endpoint + `SYS_RECV` first | Larger ABI; EL1 port unblocks K1 evidence first |
| `time` imports `sched` directly | Skips the port; reopens every handler to sched |
