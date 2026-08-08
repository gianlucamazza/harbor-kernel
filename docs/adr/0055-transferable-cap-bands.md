---
id: 0055
title: K3 — transferable capability bands
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0017, 0030, 0041, 0053, 0054]
---

# ADR-0055: Transferable capability bands

## Acceptance status

**Accepted** (2026-08-08), on delegated authority (excellence review 2026-08-08,
findings F-3/F-11/F-12; owner delegated acceptance of the review's needs-ADR
remediations).

## Problem

`sched::transfer_held` moved _any_ held CapId — endpoint, IRQ cap, task-cap —
with no band filter. Two contradictions followed:

1. ADR-0053 declares delegation chains a non-goal and ADR-0054 lists them as a
   residual, yet moving a task-cap through mode 2 _is_ delegation: A holding
   B's task-cap could hand it to C, giving C install authority over B. The
   permission lived in a code comment, not a decision.
2. `SECURITY.md` states "no transfer/revoke of IRQ caps" as a residual, while
   an `0x8000` cap moved freely. ADR-0030's single-armer model was designed
   for a fixed holder; a runtime handover was untested and undocumented.

## Decision

Only **IPC endpoint caps** are transferable. `transfer_held` refuses a moved
object whose index carries the task-cap band (`0x4000`) or the IRQ band
(`0x8000`) with `TransferError::Untransferable`. EL0 observes the existing
`Status::Authority` — the ABI surface is unchanged.

This restores ADR-0053's non-goal as enforced behaviour and makes
`SECURITY.md`'s IRQ-cap residual true again. Delegation/attenuation, if ever
wanted, arrives as a successor ADR with an attenuation model — not as an
absence of a check.

## Non-goals

- Typed transfer (declaring what kind of cap a slot expects).
- Receiver consent / notification (peer transfer stays push; recorded in
  `SECURITY.md` row 8).

## Gates

| Check                               | Evidence                                            |
| ----------------------------------- | --------------------------------------------------- |
| Host: task-cap and IRQ-band refuse  | `kernel_core` + sched-path unit coverage via oracle |
| QEMU: endpoint transfer still works | `el0-xfer-peer: ok` unchanged                       |
