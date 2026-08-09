---
id: 0063
title: Capability slots as a pure table
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0017, 0055, 0058, 0062]
amended: 2026-08-09
---

# ADR-0063: Capability slots as a pure table

## Acceptance status

**Accepted** (2026-08-09), on delegated authority (architectural improvement
plan, move 3; owner delegated acceptance per the approved plan).

## Problem

The per-task capability slots — the whole of what an EL0 agent may name
(ADR-0017 §2) — live as an array field inside `sched`'s TCBs, and the
operations that decide over them (install occupancy, transfer source/dest
arithmetic, the ADR-0055 band filter, drain on exit) are written in the
kernel binary. That is authority-deciding logic outside the host-test and
mutation net — the exact gap ADR-0058 §2 exists to close, and the same shape
`taskcap.rs` had before the excellence review flagged it (F-7).

## Decision

`kernel_core::capslots::Table<TASKS, SLOTS>` owns the slot storage and every
slot **decision**:

- `get` / `holds` — resolve a slot (`cap::from_slot` stays the one bound
  check) and membership.
- `seed` — a spawn's initial table, holes included.
- `install` — refuse out-of-range and occupied slots.
- `transfer` — source resolution, the ADR-0055 endpoint-band filter
  (`CapId::classify`, ADR-0059's one decoder), destination bounds and
  occupancy, the same-slot no-op. Returns the moved cap.
- `drain` — take and clear a task's row on exit, for hold release.

> **Amendment (2026-08-09, reconciliation per ADR-0058 — same-day, reconciled
> by the landing commit `cdcc9b7`).** The API also exposes `transfer_bounds`,
> the bounds half of `transfer` callable on its own: the ABI refuses
> out-of-range slots *before* the kernel's destination-liveness check, so the
> kernel asks bounds → liveness → full transfer — one owner, called twice.

The kernel keeps what a pure table cannot know: **who** is asking (current
task), whether the destination task is live (`Tasks::state`, epoch-checked
per ADR-0062), and the IRQ mask around the whole operation. `sched` names
tasks by slot index into the table only after validating the id — the table
itself never sees a `TaskId`.

Refusal classes are unchanged (`TransferError`, `InstallError` — the ABI
detail codes of ADR-0061 map exactly as before); `BadToTask` remains a
kernel-side refusal because liveness is not slot arithmetic.

## Gates

| Check                           | Evidence                                                                    |
| ------------------------------- | --------------------------------------------------------------------------- |
| Every refusal class host-tested | unit tests in `kernel_core::capslots`                                       |
| The decision is mutated         | `capslots.rs` in `run-mutants.sh` FILES the commit it is born (ADR-0058 §2) |
| Behaviour unmoved end to end    | boot oracle transfer/refusal assertions stay green unchanged                |
