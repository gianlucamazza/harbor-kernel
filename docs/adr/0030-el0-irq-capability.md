---
id: 0030
title: K1 remainder — EL0 IRQ notification capability and SYS_WAIT_IRQ
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0008, 0017, 0022, 0028]
---

# ADR-0030: EL0 wait on IRQ via a granted notification capability

## Acceptance status

**Accepted** (2026-08-07). Second slice of completeness track **K1**: an EL0
agent may park until a granted IRQ cookie signals, naming authority only by
**slot index** (ADR-0017). Builds on EL1 `wait_for_irq` ([ADR-0028](0028-wait-on-irq.md)).

## Context

ADR-0028 shipped the wake path (arm / signal / pending / `sched::wait_for_irq`)
and a timer producer. EL0 still had no way to name a cookie without ambient
kernel knowledge. [ADR-0008](0008-irq-handler-policy.md) already said a future
capability names the **cookie**, not a GIC id.

Roadmap K1 remainder and `SECURITY.md` residual “IRQ notification capabilities”
block calling K1 complete at the EL0 boundary.

## Decision

### 1. Separate IRQ-notification table (not IPC endpoints)

- Pure table in `kernel_core::irqcap`: mint cookie → `CapId`, lookup → cookie.
- `CapId` indices for IRQ objects use a **high half** (`0x8000 | local_index`)
  so they never collide with IPC endpoint indices (`0..MAX_ENDPOINTS`).
- Kernel façade `irq::cap` holds the global; mint only at bootstrap for this
  slice (timer cookie `1`).

### 2. `CapRights::IRQ`

A distinct rights bit. `SEND`/`RECV` do not imply wait-on-IRQ; IRQ caps do not
imply mailbox send/recv. IPC lookup still requires SEND/RECV only.

### 3. `SYS_WAIT_IRQ = 6`

| In | Out |
| --- | --- |
| `x0` = slot index | `x0` = [`Status`](../../crates/kernel-core/src/syscall.rs) |

No payload. Status mapping:

| Condition | Status |
| --- | --- |
| Slot empty / OOB / not held / not a live IRQ cap | `Authority` |
| Cookie or task already armed (ADR-0028 one-waiter) | `Busy` |
| Armed, parked, woken (or pending already set) | `Ok` |

Park reuses `sched::wait_for_irq` after cookie resolution — same mask discipline
as `SYS_RECV` (ADR-0022): never arm/block under a session-step `without_irqs`.

### 4. Grant path (first slice)

Bootstrap mints one timer-cookie notification and installs it in the oracle
task’s slot table. Manifest grants of IRQ caps are **not** required for this
slice (may follow).

### 5. Non-goals of this ADR

- Multiple waiters per cookie (still ADR-0028).
- Dynamic `irq::register` from agents.
- Cap transfer / revoke (K3).
- Cancel of an IRQ park (K2 / cancel extension).
- Second peripheral / full driver-as-agent IRQ story (K9).
- Replacing UART RX ring with wait-only.

## Consequences

### Positive

- EL0 cannot invent a cookie; it can only wait on what it was granted.
- K1 remainder has a QEMU-visible path (`el0-irq: woke`).
- Layering: handlers still only `signal`; EL0 never imports GIC ids.

### Costs

- Second object table beside IPC (documented CapId partition).
- New syscall immediate; `SECURITY.md` / `doc-claims` must stay in lockstep.

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Pack cookie as raw `CapId` without a table | Forgeable; collides with IPC lookup |
| Overload `SYS_RECV` as IRQ wait | ADR-0028 rejected IRQ-as-mailbox |
| Ambient `wait_for_irq` from EL0 with cookie in a register | Breaks slot-indexed authority |

## Gates

- Host: `irqcap` mint/lookup; `CapRights::IRQ` disjoint; `decode(6) = WaitIrq`.
- QEMU: after EL1 timer wait evidence, EL0 `SYS_WAIT_IRQ` succeeds
  (`el0-irq: woke`); empty-slot path prints `el0-irq: refused` and counts
  authority.
- `make doc-claims` — `SYS_WAIT_IRQ` row in SECURITY.
