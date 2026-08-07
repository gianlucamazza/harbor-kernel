---
id: 0034
title: K9 first slice — RNG200 as second driver-as-agent (page map)
status: accepted
date: 2026-08-07
accepted: 2026-08-07
related: [0013, 0023, 0026, 0028, 0030]
---

# ADR-0034: Second driver-as-agent — RNG200 page map (K9 entry)

## Acceptance status

**Accepted** (2026-08-07). First slice of completeness track **K9**: a second
peripheral beyond PL011 is granted to an agent as a **named Stage-1 page**
([ADR-0013](0013-narrow-device-windows.md)), used from EL0, and revoked by AS
destroy (kill).

Does **not** require live entropy on every platform: QEMU `raspi4b` has no
RNG200 backend; absence is an allowed outcome if the map/kill path still runs.

## Context

Roadmap K9: “Second peripheral on the M6 pattern; IRQ-cap path.” PL011 already
proves map + kill + RX ownership. A second device proves the pattern is not
UART-specific.

RNG200 is already named in memmap and has an EL1 polled driver. It needs no
IRQ for a first slice (FIFO poll / register load). Timer-as-agent would prove
IRQ caps without MMIO; that remains a later K9 slice.

## Decision

### 1. Named agent window

| Symbol | Value |
| --- | --- |
| `RNG200_BASE` | existing BSP PA |
| `RNG200_REG_BYTES` | one Stage-1 page (`FRAME_SIZE`) |
| `USER_RNG_VA` | fixed user VA, disjoint from PL011 and user RAM window |

### 2. Agent body (oracle)

1. `Agent::create_prepared` + `map_device_page(USER_RNG_VA, RNG200_BASE, USER_RW)`.
2. EL0: load `RNG_CTRL` at offset 0, then `SYS_PING`.
3. Outcomes accepted on the good path:
   - successful load + ping → `rng-agent: map read ok`
   - data abort / fault on missing backend → `rng-agent: map fault ok` (QEMU)
4. `destroy` → `rng-agent: killed ok` with frame pool report (same spirit as PL011).

### 3. Non-goals of this ADR

- Entropy quality / CSPRNG claims.
- IRQ notification for RNG (polled device).
- SPI / XPT2046 / #14.
- Third peripheral; general driver framework.

## Consequences

### Positive

- K9 has a second named device agent on the M6 template.
- QEMU and silicon share the map/kill path; silicon can add a strict read-ok
  stamp later without changing the shape.

### Costs

- QEMU oracle must accept fault-on-absent as success for the map probe.
- Two user Device VAs to keep disjoint in memmap.

## Gates

| Check | Evidence |
| --- | --- |
| Map + kill | boot-check `rng-agent: killed ok` |
| EL0 use of window | `rng-agent: map read ok` **or** `rng-agent: map fault ok` |
| No 16 MiB agent blanket | review / ADR-0013 |

## Alternatives rejected

| Alternative | Why not |
| --- | --- |
| Timer-only agent first | Skips M6 map pattern |
| SPI / touch | Collides with #14 / debug-display; larger surface |
| Require live FIFO on QEMU | Emulator has no backend; would force fake MMIO |
