---
id: 0060
title: Syscall reply layer as a pure machine
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0017, 0022, 0039, 0041, 0042, 0054, 0059]
---

# ADR-0060: Syscall reply layer as a pure machine

## Acceptance status

**Accepted** (2026-08-08), on delegated authority (architectural improvement
plan, move 1; owner delegated acceptance per the approved plan).

## Problem

The reply mappers in `src/agent` are the security-visible semantics of the
kernel — which subsystem error becomes which `Status`, which counter bumps,
which reply registers get written — and they were pure functions welded to
`sched::`/`ipc::` statics. Zero unit tests; evidence only via boot oracles;
`verification.md` had already conceded the gap (`RecvError::Busy → Busy`
reachable by nothing). This is the highest-value slice of the src/-untestable
asymmetry: small, pure by nature, and exactly where an ABI bug would live.

## Decision

`kernel_core::reply` owns the mapping. Per call, an **outcome enum** (what the
subsystem answered, stripped of kernel types) and one pure function
`outcome → Reply`:

```rust
pub struct Reply {
    pub status: Status,
    /// x1..x3, written only when present (recv payload).
    pub payload: Option<[u64; 3]>,
    pub delta: StatDelta,   // which SessionStats fields bump, by how much
}
```

`src/agent` shrinks to marshalling: read GPRs → call subsystem → convert its
`Result` into the outcome enum (mechanical, one arm per variant) → pure map →
write GPRs, apply delta, resume. Name unpacking for `SYS_RESOLVE`
(`(len, packed) → [u8; 8]`) moves too — it is arithmetic.

The tests in `kernel_core::reply` **are** the authority table's semantic half:
every outcome variant asserts its `Status`, its payload behaviour (no payload
on refusal — the kernel does not clear an agent's own registers), and its
exact counter. `doc-claims` keeps checking the set; these tests check the
adjective it cannot.

## Non-goals

- Changing any mapping (byte-for-byte behaviour preserved; the boot oracle's
  exact-count assertions are the witness).
- Moving the slot/hold/grant _lookups_ — they need kernel state and stay in
  `src/agent`/`sched`; what moves is the decision after the answer.
- The refusal taxonomy (move 6) — a successor builds it on this layer.

## Gates

| Check                       | Evidence                                                                   |
| --------------------------- | -------------------------------------------------------------------------- |
| Mapping is total and tested | host tests in `kernel_core::reply`, one per outcome variant                |
| Behaviour unchanged         | boot oracle exact counts (`ipc: refuse count=6`, `el0-*` lines) stay green |
| Mutation                    | `reply.rs` joins the mutation file list (ADR-0058 §2)                      |
