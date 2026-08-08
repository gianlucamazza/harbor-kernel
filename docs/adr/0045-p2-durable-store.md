---
id: 0045
title: P2 residual — durable keyed store region (soft-reboot / reload path)
status: accepted
date: 2026-08-08
accepted: 2026-08-08
related: [0036]
---

# ADR-0045: Durable store region (P2 residual entry)

## Acceptance status

**Accepted** (2026-08-08). Residual of **P2**: product blobs can be written to a
**dedicated image-resident durable region** and reloaded from raw bytes without
host inject of that payload — advancing past the RAM-only table in ADR-0036.

## Decision

### 1. Pure `kernel_core::durable`

Wire format for a small multi-blob block in a fixed-capacity byte array:
magic, version, count, then records (key + payload). Host-tested encode/decode
round-trip and refuse paths.

### 2. Placement

Linker section `.durable_store` (4 KiB), **NOLOAD**, **outside `.bss`** so
`boot.s` does not clear it on soft reset. First cold power-on may be zero;
writers treat bad magic as empty.

### 3. EL1 façade `src/durable`

- `put(key, payload)` — read-modify-write encode of the durable section
  (decode existing blobs if magic valid, merge/replace key, re-encode).
- `get(key, out)` — decode the section and copy the matching payload.

No separate RAM cache: the section **is** the store. Callers that need a
soft-reload story re-`get` from the region after other state is discarded.

### 4. Oracle

`put(b"cfg", b"persist")` → `get` same key from the section →
`durable: reloaded` (proves region round-trip without host inject of that payload).

### 5. Residuals

- True SD/eMMC media and power-cycle durability on Pi (`done (HW)`).
- EL0 storage caps.
- Explicit wipe-then-reload API (optional; not required for this slice).
- QEMU full `system_reset` may still wipe RAM — section put/get is the QEMU bar.

## Gates

| Check | Evidence |
| --- | --- |
| Host encode/decode | unit tests |
| QEMU put then get from section | `durable: reloaded` |
