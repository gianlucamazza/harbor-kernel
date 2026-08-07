---
id: 0035
title: P5 first slice — EL1 name registry (name → CapId)
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0017, 0021, 0026, 0032]
---

# ADR-0035: Name registry for endpoint discovery (P5 entry)

## Acceptance status

**Accepted** (2026-08-07). First slice of completeness track **P5**: a creator
can **bind** a short name to a `CapId` and **resolve** it later without embedding
raw capability bits in agent programs or oracle tables.

## Context

Compositions hard-wire CapIds or slot conventions between spawn sites. Product
bar item 7: “endpoints findable without hard-coded oracle wiring.” Full naming
(dynamic advertise, multi-node) is out of scope; a bootstrap-owned registry is
enough to break hard-wiring for the first services.

## Decision

### 1. Pure table `kernel_core::naming`

- Fixed capacity (`MAX_NAMES`, `MAX_NAME_LEN`).
- `bind(name, cap)` — insert or replace; refuse empty/too-long names / full table.
- `resolve(name) → CapId` — exact byte match; missing → error.
- `unbind(name)` — remove binding.
- No MMIO; host-tested.

Names are opaque byte strings (not UTF-8 validated in the table). Creators
choose conventions (e.g. `b"console"`).

### 2. Kernel façade `src/naming`

Global table under IRQ mask (same pattern as IPC). Trusted EL1 only for this
slice — no EL0 syscall to bind/resolve.

### 3. Relation to revoke

The registry stores **CapId bits**, not a live check. After
[`revoke_channel`](0032-k3-channel-revoke.md), resolve may still return the old
id; send/recv then refuse. Creators should `unbind` after revoke if they want
`missing`. Auto-unbind on revoke is a later slice.

### 4. Non-goals

- EL0 `SYS_RESOLVE` / name caps.
- Hierarchical paths, wildcards, directories.
- Capability transfer of names between agents.
- Automatic sync with channel lifecycle.

## Gates

| Check | Evidence |
| --- | --- |
| Host bind/resolve/unbind | unit tests |
| QEMU bind + resolve | `name: resolved` |
| QEMU missing | `name: missing` |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Only document conventions | Still hard-wired CapIds in code |
| EL0 syscall first | Ambient discovery surface before creator policy |
| Store only endpoint index without generation | Breaks after recycle |
