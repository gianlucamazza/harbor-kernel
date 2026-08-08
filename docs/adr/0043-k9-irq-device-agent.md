---
id: 0043
title: K9 residual — IRQ-cap device agent (wait path)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0030, 0034]
---

# ADR-0043: IRQ-cap device agent (K9 residual entry)

## Acceptance status

**Accepted** (2026-08-08). Residual of **K9**: a **driver-as-agent** that is
granted only an IRQ notification cap and completes a product wait path via
`SYS_WAIT_IRQ` — not merely mapping a Device page ([ADR-0034](0034-k9-rng-driver-agent.md)).

## Context

RNG first slice proved page map + kill. Wait-on-IRQ was proven on lab demos
([ADR-0030](0030-el0-irq-capability.md)). The residual is a **named agent role**:
device-shaped task whose authority is the IRQ cap.

## Decision

1. The product path for an IRQ-cap-only wait is `SYS_WAIT_IRQ` on a granted
   notification (timer cookie for this slice).
2. Bootstrap proves it on the same sequential path as the K1 EL0 wait
   (`irq_wait_task`): EL1 wait, then EL0 `SYS_WAIT_IRQ` — **one waiter per
   cookie**, no concurrent second arm.
3. Oracle lines: `el0-irq: woke` and `irq-device: woke` (same wake; the second
   names the K9 residual product story).

Cookie remains the arch timer. A future slice may bind a non-timer SPI/UART
cookie and a dedicated task once multi-waiter IRQ tables exist.

### Non-goals

- New hardware lines beyond the timer for this slice.
- Multi-waiter IRQ table changes.

## Gates

| Check | Evidence |
| --- | --- |
| QEMU device agent wait | `irq-device: woke` |
