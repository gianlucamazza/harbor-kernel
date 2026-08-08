---
id: 0056
title: IPC ABI capacities — canonical numbers
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0017, 0054]
---

# ADR-0056: IPC ABI capacities (successor to ADR-0017 §4 numbers)

## Acceptance status

**Accepted** (2026-08-08), on delegated authority (excellence review 2026-08-08,
finding F-4; owner delegated acceptance of the review's needs-ADR remediations).

## Problem

ADR-0017 §4 froze mailbox count (8) and endpoint count (16) as part of the EL0
ABI. The code then moved twice without a successor — 8/16 → 12/24 (`98fb538`),
→ 16/32 (`0cee6e4`, oracle boot-path pressure) — leaving two owners of one
fact in disagreement, and demo pressure silently reshaping a product ABI.

## Decision

The canonical capacities are the table below. This ADR — not ADR-0017 §4, not
a code comment — owns them; `src/ipc/mod.rs` restates them and a `doc-claims`
row compares the two mechanically.

| Constant        | Value |
| --------------- | ----- |
| Mailbox depth   | 4     |
| `MAX_MAILBOXES` | 16    |
| `MAX_ENDPOINTS` | 32    |

Any future change lands as a successor ADR **before** the code, regardless of
what motivates it. "The oracle needed more channels" is a reason to write the
successor, not to skip it.

## Gates

| Check                      | Evidence                                                           |
| -------------------------- | ------------------------------------------------------------------ |
| Constants match this table | `make doc-claims` (new check, seen red against a planted mismatch) |
