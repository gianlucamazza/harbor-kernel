---
id: 0042
title: K2 residual — EL0 SYS_RECV_TIMEOUT
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0022, 0040]
---

# ADR-0042: EL0 recv with timeout (K2 residual entry)

## Acceptance status

**Accepted** (2026-08-08). Residual of **K2**: an agent may park on recv with a
tick deadline via a dedicated syscall, reusing [ADR-0040](0040-k2-park-timeout.md).

## Decision

### `SYS_RECV_TIMEOUT` = imm 9

| register | meaning |
| --- | --- |
| `x0` | RECV slot |
| `x1` | timeout in monotonic ticks (`≥ 1`) |
| `x0` out | Status (`Ok` / `Cancelled` / …) |
| `x1..x3` out | payload if `Ok` |

Kernel path: `ipc::recv_with_timeout` after slot→CapId. Timeout →
`Status::Cancelled` (same as EL1).

### Non-goals

- Folding timeout into `SYS_RECV` via flag (kept as separate imm for clarity).
- Distinct `TimedOut` ABI status.

## Gates

| Check | Evidence |
| --- | --- |
| Decode(9) | unit test |
| QEMU agent timeout without sender | `el0-timeout: cancelled` |
