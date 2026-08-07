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

### 1. IRQ wait port (`irq::wait` + `kernel_core::irqwait`)

- **One waiter per cookie**; second arm on a busy cookie or busy task is
  **refused** (host-tested pure table — no overwrite).
- `arm(cookie, task)` — voluntary path, before park.
- `signal(cookie)` — IRQ path only: clear arm, set pending, push `task` onto
  the single SPSC wake queue owned by `irq`.
- `drain` — voluntary path (`sched::poll_wakes`) → `wake_task`.
- `take_pending` — closes the lost-wakeup window between arm and block.

Handlers call **`irq::wait::signal(cookie)` only** (no `sched` import). Timer
and UART cookies are 1 and 2 at registration.

### 2. `sched::wait_for_irq(cookie)`

EL1 API (bootstrap / driver tasks; EL0 IRQ cap is a successor ADR):

1. `arm(cookie, current)` — fail loud if busy
2. If `take_pending()` → return (IRQ already posted)
3. `block_current()`
4. Clear pending / arm on resume

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

### Negative

- EL0 still cannot name an IRQ wait without a later ABI ADR (explicit successor,
  not a silent half-feature).

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
