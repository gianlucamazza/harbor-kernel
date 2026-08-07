---
id: 0027
title: H1 first slice — external agent store at a fixed physical address
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0021, 0023]
---

# ADR-0027: External agent store (H1 entry)

## Acceptance status

**Accepted** (2026-08-07). First structural step of vision **H1** (composition
runtime): agent text may arrive from **outside** the kernel `.rodata`, not only
from `const` arrays compiled into the image.

## Context

[ADR-0021](0021-agents-as-data-and-the-manifest.md) made agents data (image +
geometry + slot indices) but kept the table as a Rust `const` in the image.
Its §4 deferred a byte format until "the bytes come from outside the image".

Vision H1 requires compositions that are not hard-wired into every build. A
full SD filesystem reader is a large surface. This ADR takes the **smallest
honest external path**:

1. A versioned **byte store** of agent records (host-tested parser).
2. A **fixed physical address** where a boot loader (QEMU `loader` device,
   later firmware or a thin SD path) places that store before entry.
3. The product loader **prefers** a valid store when present; otherwise it
   falls back to the built-in beacon (M8).

This is not "load from network" and not "parse untrusted internet input". The
store is trusted boot input, like `kernel8.img` itself, until a later ADR adds
signing or an untrusted-input policy.

## Decision

### 1. Wire format (`kernel_core::agentstore`)

Little-endian. Host-tested pure parse (no MMIO).

| Field | Size | Notes |
| --- | --- | --- |
| magic | 4 | `b"HARB"` |
| version | 4 | `1` |
| count | 4 | agents, `1..=MAX_AGENTS` |
| reserved | 4 | zero |
| *per agent* | | |
| name | 16 | UTF-8, NUL-padded |
| text_pages | 4 | ≥ 1 |
| stack_pages | 4 | ≥ 1 |
| slots | 4 | each byte is held-index, or `0xFF` empty |
| reserved | 4 | zero |
| image_len | 4 | bytes of text |
| image | `image_len` | then pad to 4-byte boundary |

No device grants in v1 (always `None`). Console grant remains index into the
loader's held list, as today.

### 2. Placement

**Superseded for load address by [ADR-0029](0029-agent-store-in-image.md).**
The store is an image section (`.agent_store`) filled by host inject — same path
on QEMU and Pi. The original fixed-PA + QEMU `loader` device is withdrawn.

### 3. Loader policy

1. If `parse` succeeds on the image store section, build the runtime manifest
   from the store and load those agents only (plus the usual held-cap list).
2. Else use the compiled-in table (product beacon / oracle beacon+mute).
3. Log which path ran: `loader: store n=… image` vs `loader: builtin`.

### 4. Non-goals of this ADR

- FAT/SD driver inside the kernel.
- Untrusted-input validation beyond structural parse.
- Device grants in the store.
- Signing.
- Collapsing the EL1 driver half (ADR-0023).

## Consequences

### Positive

- Product compositions can change without rebuilding the kernel binary.
- Parser risk is confined to `kernel-core` and host tests.
- Oracle path unchanged when no store is present.

### Negative

- Name strings and image slices borrow the store for the life of the boot —
  the store must not be overwritten (inject is boot input).
- Placement details: see ADR-0029.

### Gates

| Reversal | Gate |
| --- | --- |
| Format drift | Host tests on `agentstore::parse` |
| Product ignores store when present | `make product-boot-check` (injected image store) |
| Fallback broken without store | `make boot-check` (oracle image, empty section → builtin) |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Full filesystem in-kernel first | Wrong size for the first H1 slice |
| Always link agents into the kernel | Does not open H1 composition |
| Network load first | Needs stack, trust model, far larger TCB |
| Device grants in v1 | Not needed for beacon-class product; add in v2 |
