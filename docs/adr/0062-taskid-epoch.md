---
id: 0062
title: Epoch in the task identity
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0006, 0038, 0053, 0057]
---

# ADR-0062: Epoch in the task identity

## Acceptance status

**Accepted** (2026-08-09), on delegated authority (architectural improvement
plan, move 2; owner delegated acceptance per the approved plan).

## Problem

`TaskId` is a bare slot index. Slots are reused after exit, so _any_ stored
task reference — a task-cap entry, a parked wake token, a pending IRQ
delivery, a `creator` field — can silently come to name a different task than
the one it was minted for. Today three mechanisms compensate:

- the task-cap table revokes on exit (ADR-0057) **and** `spawn` re-checks the
  freshly admitted slot for a leaked live task-cap (`STALE-TASKCAP`, ADR-0057
  §1) — a cross-check for a state that should be unreachable;
- the IRQ wait table's pending bitmap is indexed by raw id, so a delivery
  posted for an exited task is consumable by the slot's next tenant;
- every site that stores a `TaskId` across a yield relies on revocation
  having run, with no way to _detect_ a stale reference.

A per-table generation in every such table would scatter the same fact
(N compensations for one missing property). The property belongs in the
identity itself.

## Decision

`TaskId` becomes `{ slot: u16, epoch: u16 }`:

- **Minting.** Only `Tasks::admit` mints a live id, carrying the slot's
  current epoch. `Tasks::live_id(slot)` re-derives it for iteration sites.
- **Bump.** `Switch::Exit` increments the slot's epoch at the moment the
  state goes `Empty`. Idle (slot 0) never exits; its epoch stays 0.
- **Validation.** `Tasks::state` and `Tasks::wake` treat an epoch mismatch as
  _no such task_ (`None` / `false`). Everything downstream that asks the
  model before acting — transfer target checks, cancel, reap — inherits the
  refusal with no extra code.
- **Transport.** `to_raw()/from_raw()` pack the id into a `u32`
  (`epoch << 16 | slot`) for the wake SPSC queue and boot demo atomics. A
  raw value is transport, not authority: an unpacked stale id fails
  validation like any other.
- **IRQ wait.** `irqwait::WaitTable` stores full `TaskId`s; the pending
  mark is `Option<TaskId>` per slot, so a delivery posted for an exited
  task is not consumable by the slot's next tenant.

### What this deletes

The `STALE-TASKCAP` spawn cross-check, its boot-oracle absence line, and
`taskcap::has_live_for` (its only consumer). A freshly admitted id has a new
epoch, so a leaked task-cap naming the previous tenant _cannot_ name the new
task — the state the cross-check watched for is unrepresentable, and a
compensating check for an unrepresentable state is debt (session rule:
structural fix over compensation). Task-cap revocation on exit (ADR-0057)
stays: it frees table entries, which is a resource concern, not aliasing.

## Bounds

The u16 epoch wraps after 65 536 exits on one slot, at which point a stale id
minted 65 536 lifetimes ago revalidates — same decided-bound shape as the
task-cap generation (ADR-0057 §3), unreachable from any current boot, and
encoded in a host test rather than left implicit.

## Gates

| Check                                        | Evidence                                                              |
| -------------------------------------------- | --------------------------------------------------------------------- |
| Stale id refused at every model entry point  | host tests in `kernel_core::tasks` (state/wake), `irqwait` (pending)  |
| Wrap bound is the decided one                | host test: 65 536 exit cycles revalidate                              |
| No reachable sequence corrupts the scheduler | `model_sched` extended with stale-id operations                       |
| Cross-check really unrepresentable           | `STALE-TASKCAP` print and oracle line deleted; boot stays green       |
| Identity is a boundary                       | `make mutants` scope includes `tasks.rs`, `runqueue.rs`, `irqwait.rs` |
