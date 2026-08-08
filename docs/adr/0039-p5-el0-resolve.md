---
id: 0039
title: P5 residual — EL0 SYS_RESOLVE into an empty slot
status: accepted
date: 2026-08-08
accepted: 2026-08-08
amended: 2026-08-08
related: [0017, 0035]
---

# ADR-0039: EL0 name resolve (P5 residual entry)

## Acceptance status

**Accepted** (2026-08-08). Residual of completeness track **P5**: an agent may
**resolve** a short name published by the creator into an **empty local slot**,
without ever seeing raw `CapId` bits ([ADR-0017](0017-el0-capability-abi.md)).

## Context

[ADR-0035](0035-p5-name-registry.md) bound/resolves only at trusted EL1. Product
bar item 7 still needed agents to **find** services without hard-wired CapIds
in spawn tables.

## Decision

### 1. `SYS_RESOLVE` = imm 7

| register | meaning |
| --- | --- |
| `x0` in | empty slot index to fill |
| `x1` in | name length (1..=8 for this slice) |
| `x2` in | name bytes packed little-endian in the low bytes of `x2` (up to 8) |
| `x0` out | [`Status`](../../crates/kernel-core/src/syscall.rs) |

On success the slot holds the resolved `CapId`. On failure the slot is unchanged.

### 2. Failure → Authority

Missing name, bad length, occupied/out-of-range slot → `Status::Authority`
(same refusal class as a bad slot). Counted as an authority refusal on the agent
stats path.

### 3. Ambient resolve (superseded)

Originally any agent could attempt resolve. **Superseded by
[ADR-0052](0052-p5-resolve-grant.md):** resolve requires a per-task grant
(`may_resolve`). Creators still control **what names exist**.

### 4. Non-goals

- Names longer than 8 bytes on the EL0 path (EL1 registry still allows 16).
- Hierarchical paths.
- Auto-unbind on revoke (still residual from ADR-0035).

## Gates

| Check | Evidence |
| --- | --- |
| Host: decode(7) is Resolve | unit test |
| QEMU: agent resolves bound name | `el0-resolve: ok` |
| QEMU: missing name refuses | `el0-resolve: refused` |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Pointer into user memory for name | Larger attack surface before needed |
| Return CapId in x1 | Breaks ADR-0017 (no CapId to EL0) |

**Amended 2026-08-08** (ADR-0058 convention): §3's ambient-resolve slice was
superseded by the per-task grant of [ADR-0052](0052-p5-resolve-grant.md)
(commit `440d77b`); the section is marked so in place.
