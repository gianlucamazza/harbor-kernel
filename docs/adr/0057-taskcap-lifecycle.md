---
id: 0057
title: K3 — task-cap lifecycle invariants
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0033, 0038, 0051, 0053, 0054]
---

# ADR-0057: Task-cap lifecycle invariants

## Acceptance status

**Accepted** (2026-08-08), on delegated authority (excellence review 2026-08-08,
findings F-5/F-25/F-28/F-29; owner delegated acceptance of the review's
needs-ADR remediations).

## Problem

A task-cap names a `TaskId`, and `TaskId` is a **recycled TCB slot index with
no generation of its own**. The cap's u16 generation protects the handle, not
the binding: if any exit path skipped `taskcap::revoke_task(exiting)`, a
surviving task-cap would silently name the slot's next occupant — an authority
escalation. ADR-0054 states the revoke call but not _why it is load-bearing_,
and nothing asserted it.

## Decision

1. **Invariant (stated, checked):** every TCB slot release runs through the
   single `switch_with(Switch::Exit)` funnel, and `taskcap::revoke_task(from)`
   runs inside it _before_ the ADR-0038 cascade wakes any child. A live
   task-cap must never name an `Empty` or re-admitted slot. `sched::spawn`
   now cross-checks at admit time: a freshly admitted `TaskId` with a live
   task-cap entry is the bug this ADR exists to catch, and it refuses loudly
   (`taskcap::has_live_for`).
2. **Mint contract:** `mint` is EL1-only and takes a task the caller has just
   spawned or otherwise knows live; the pure table cannot check liveness and
   does not pretend to. Entries are freed only by `revoke_task(target)` —
   there is no per-cap free. Consequence: `MAX_TASK_CAPS` (32) <
   `MAX_TASKS` (40) is a deliberate pressure bound, and a `mint` failure is
   `MintError::Full`, observable (the oracle asserts its absence on the boot
   path).
3. **Generation wrap:** the u16 generation wraps after 65 535 mint cycles on
   one local index, at which point a stale handle re-validates. Unreachable
   from the current boot; stated here so the bound is a decision, not a
   surprise. Host-tested at the wrap boundary.

## Residuals (registered in ADR-0049)

- **Spawn epoch** in the task-cap entry (binds the cap to an admission epoch,
  not just a slot) — required **before K8 SMP**, where the
  `switch(Exit)` → `revoke_task` window becomes a real race. ADR-0051's
  re-audit list carries the same entry.
- Per-cap revoke / force-kill / control rights (unchanged from ADR-0054).

## Gates

| Check                               | Evidence                                    |
| ----------------------------------- | ------------------------------------------- |
| Stale-after-exit refused end-to-end | QEMU `xfer-peer: stale refused`             |
| Move-not-copy                       | QEMU `el0-xfer-peer: donor emptied`         |
| No silent mint exhaustion on boot   | boot-check asserts absence of `mint FAILED` |
| Wrap boundary                       | host test in `kernel_core::taskcap`         |
