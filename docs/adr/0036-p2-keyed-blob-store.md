---
id: 0036
title: P2 first slice — EL1 keyed blob store (on-target put/get)
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0026, 0027, 0029, 0035]
---

# ADR-0036: Keyed blob store for on-target load/persist (P2 entry)

## Acceptance status

**Accepted** (2026-08-07). First slice of completeness track **P2**: the product
path can **put** and **get** opaque blobs by short key **at runtime on the
target**, without a host rebuild/inject of that payload. Cross-reboot media
(SD/eMMC) remains residual.

## Context

H1 product bar item 3: load without rebuild-only demos. **K6** already loads a
composition from an image-resident agent store that the **host** injects
([ADR-0027](0027-h1-external-agent-store.md) /
[ADR-0029](0029-agent-store-in-image.md)). That is not on-target persist: the
machine never writes durable product data itself.

A full block stack or POSIX FS is out of scope for H1 entry. The smallest honest
step is a **creator-owned keyed blob table** the kernel can put/get under
trusted EL1 (same creator pattern as the P5 name registry), with pure logic
host-tested and a QEMU oracle that proves round-trip and missing.

## Decision

### 1. Pure table `kernel_core::storage`

- Fixed capacity (`MAX_BLOBS`, `MAX_KEY_LEN`, `MAX_PAYLOAD`).
- `put(key, payload)` — insert or replace; refuse bad keys / oversized payload /
  full table.
- `get(key, out)` — copy payload into caller buffer; missing / bad key / short
  buffer → error.
- `delete(key)` — remove; missing → error.
- No MMIO; host-tested.

Keys are opaque byte strings (exact match). Payloads are opaque bytes — not
parsed as agent stores in this slice.

### 2. Kernel façade `src/storage`

Global table under IRQ mask (IPC/naming pattern). Trusted EL1 only — **no EL0
storage syscall** in this slice. The backing memory is the pure table's embedded
payload arrays (RAM within the boot). That is enough to prove **on-target**
write-then-read without host inject of the oracle payload.

### 3. Relation to agent store (K6)

Agent-store inject remains the composition **load** path. This store is a
separate product service for small durable-shaped data (config, seals, tokens).
Loading agents **from** this store, or flushing it to SD, is a later slice.

### 4. Non-goals / residuals

- SD/eMMC/NVMe block driver and media that survive reboot.
- Filesystem, directories, large files, mmap.
- EL0 storage caps / syscalls.
- Automatic bind into the name registry.
- Hardware stamp (`done (HW)`).

## Gates

| Check | Evidence |
| --- | --- |
| Host put/get/delete / refuse paths | unit tests on `kernel_core::storage` |
| QEMU put then get round-trip | `store: got` |
| QEMU missing key | `store: missing` |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Only improve host inject (K6) | Still rebuild/inject-only; no on-target persist |
| Full FS or FAT reader first | Wrong size; no TCB budget for H1 first slice |
| Make `.agent_store` writable and rewrite agents | Couples composition format to config storage; larger blast radius |
| EL0 syscall first | Ambient storage surface before creator policy |
