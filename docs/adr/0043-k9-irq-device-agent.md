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

1. Bootstrap mints (or reuses) a timer IRQ notification into slot 0 of a
   dedicated task.
2. That task runs an EL0 program: `SYS_WAIT_IRQ` then `SYS_EXIT`.
3. Oracle line: `irq-device: woke` (distinct from lab `el0-irq: woke`).

Cookie remains the arch timer (line already registered). A future slice may
bind a non-timer SPI/UART cookie the same way.

### Non-goals

- New hardware lines beyond the timer for this slice.
- Multi-waiter IRQ table changes.

## Gates

| Check | Evidence |
| --- | --- |
| QEMU device agent wait | `irq-device: woke` |
