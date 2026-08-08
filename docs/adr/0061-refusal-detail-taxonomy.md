---
id: 0061
title: Refusal detail taxonomy in x1
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0017, 0022, 0060]
---

# ADR-0061: Refusal detail taxonomy in `x1`

## Acceptance status

**Accepted** (2026-08-08), on delegated authority (architectural improvement
plan, move 6; owner delegated acceptance per the approved plan).

## Problem

Slot out-of-range, empty slot, wrong band, name-not-found, missing resolve
grant, bad transfer dest — all collapse to `Status::Authority = 1`. An agent
cannot distinguish "I miscounted my own slots" from "policy refused me", which
matters exactly when real agents get written. The merged counter is asserted
exactly by the boot oracle, which ossifies the merge.

## Decision

`x0` keeps the **class** (`Status`, unchanged — every existing agent's check
still works). On a refusal, `x1` carries a **detail code**; `x2`/`x3` stay
untouched.

This deliberately amends the canonical ABI statement in
`kernel_core::syscall`'s module doc ("`x1..x3` carry a payload only when `x0`
is `Ok`" — a code-owned rule; no ADR ever stated it): a refusal _reason_ is a
reply, not stale kernel data. The
amendment is additive — an agent that ignores `x1` behaves exactly as before.

Codes (`kernel_core::reply::RefusalDetail`, stable numbering, 0 = none):

| Code | Name             | Produced by                                             |
| ---- | ---------------- | ------------------------------------------------------- |
| 1    | `BadCap`         | send/recv/timeout: slot empty, OOB, stale, wrong rights |
| 2    | `UnknownDest`    | transfer `x2` not 0/1/2                                 |
| 3    | `BadFromSlot`    | transfer source empty/OOB                               |
| 4    | `BadToTask`      | transfer target dead/unknown/self/stale task-cap        |
| 5    | `ToSlotFull`     | transfer destination occupied                           |
| 6    | `ToSlotOob`      | transfer destination index out of range                 |
| 7    | `Untransferable` | moved object not an endpoint cap (ADR-0055)             |
| 8    | `NoGrant`        | resolve without `may_resolve` (ADR-0052)                |
| 9    | `BadNameLen`     | resolve name length outside 1..=8                       |
| 10   | `Missing`        | resolve name not bound                                  |
| 11   | `BadSlot`        | resolve install slot occupied/OOB                       |
| 12   | `NotIrqCap`      | wait-irq slot not a held IRQ notification               |

The mapping lives in `kernel_core::reply` (ADR-0060), host-tested per
variant; per-cause counters stay a residual — the machine-wide counter and
the boot oracle's exact assertions are unchanged.

## Gates

| Check                                  | Evidence                                 |
| -------------------------------------- | ---------------------------------------- |
| Every refusal variant maps to its code | host tests in `kernel_core::reply`       |
| Class unchanged                        | boot oracle exact counts stay green      |
| Detail observable end to end           | oracle `el0-xfer-peer: refused refusals=… detail=4` |
