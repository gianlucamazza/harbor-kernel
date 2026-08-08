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

- `put` / `get` / `flush` — update pure table then encode into the section.
- `reload` — parse section into table.

### 4. Oracle

`put` → wipe in-memory table → `reload` from section → `durable: reloaded`.

### 5. Residuals

- True SD/eMMC media and power-cycle durability on Pi (`done (HW)`).
- EL0 storage caps.
- QEMU full `system_reset` may still wipe RAM — soft-reload is the QEMU bar.

## Gates

| Check | Evidence |
| --- | --- |
| Host encode/decode | unit tests |
| QEMU reload after wipe | `durable: reloaded` |
