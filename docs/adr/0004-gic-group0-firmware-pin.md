---
id: 0004
title: GIC Group 0 with IAR/EOIR, and the firmware pin it depends on
status: accepted
date: 2026-08-04
accepted: 2026-08-04
---

# ADR-0004: GIC Group 0 with IAR/EOIR, and the firmware pin it depends on

## Acceptance

**Accepted 2026-08-04.** Verified on a Pi 4B with the pinned firmware
(`soft_ticks`, `HPPIR=30`, `IAR=0x1e`, `selftest: OK` under `--features
bringup`, and live `ticks=` after early MMU). Blob hashes are enforced by
`fetch-blobs.sh` (**seen red** on a corrupted expected hash). Immutable under
the ADR lifecycle; revisit on every `firmware_tag` bump as the ADR already
requires.

## Context

`drivers/gicv2.rs` programs the PPIs in **Group 0** and claims/EOIs through
**`IAR`/`EOIR`**, not through the Group 1 aliased registers.

This choice is **empirical**, not derived from the manual: during M1 bring-up,
`HPPIR` reported PPI 30 as pending but claiming through Group 1 did not advance
the ticks. In the Non-Secure view of GICv2, bit 0 of `GICD_CTLR` is
`EnableGrp1`, so the sequence that works depends on the state `start4.elf` leaves
the distributor in.

It is the only dependency on closed firmware the kernel has on the hot path, and
it is **passive**: inherited state, not a protocol. The other two of the same
nature are `CNTFRQ_EL0` (read, not set) and the PL011 clock (48 MHz assumed, with
`enable_uart=1` and `core_freq_min=500`).

## Decision

Keep Group 0 + `IAR`/`EOIR`, and **tie this choice explicitly to the firmware
pin**: `firmware_tag=1.20250430`, with the hashes in `EXPECTED.sha256` verified
before installation.

Bumping the tag is a deliberate change that requires **re-running the bring-up
gates on hardware**, because a regression here does not produce an error: it
produces a boot that reaches the console and never prints `ticks=`.

## Consequences

**Positive** — the path is verified on silicon with this firmware (`HPPIR=30`,
`IAR=0x1e id=30`, `ticks 0 -> 2`, `selftest: OK`), not merely assumed.

**Negative** — the kernel is not portable to arbitrary firmware without
re-validation. The constraint is documented in [`blobs.md`](../blobs.md) rather
than left implicit in the driver.

## Alternatives considered

| Alternative                            | Why not                                                                   |
| -------------------------------------- | ------------------------------------------------------------------------- |
| Group 1 + `AIAR`/`AEOIR`               | Tried during M1: `HPPIR` saw the PPI, the claim did not advance the ticks |
| Reprogram the distributor from scratch | Requires knowing the secure-side state, which we do not have              |
| Do not pin the firmware                | Would keep this dependency invisible until it breaks                      |

## The gate that protects this decision

The `--features bringup` gates (`make bringup-builds` guarantees they compile;
running them requires hardware). Verified on a Pi 4B Rev 1.5 on 2026-08-04, after
the move to an early MMU changed the memory regime underneath them.

`scripts/fetch-blobs.sh` refuses blobs whose hashes do not match the committed
ones — **seen red** by corrupting an expected hash.

## When to revisit

On every `firmware_tag` bump, and on any board whose EEPROM leaves the GIC in a
different state. The regression signal is the absence of `ticks=`, not an error.

## References

`src/drivers/gicv2.rs`, `src/bootstrap/selftest.rs`, [`blobs.md`](../blobs.md),
[`interrupts.md`](../interrupts.md).
