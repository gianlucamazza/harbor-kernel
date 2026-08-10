---
id: 0073
title: Discovery first slice — FDT reader and the discover report
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0011, 0065, 0072]
---

# ADR-0073: Discovery first slice (FDT reader + `discover:` report)

## Acceptance status

**Accepted** (2026-08-09, on delegation from Gianluca). Implements the first
slice of [ADR-0072](0072-hardware-self-discovery-design.md): parse the
RO-mapped device tree, reconcile against compiled expectations, print the
`discover:` report on AArch64. No x86 change, no display probing, no map
consumption.

## Decision

### 1. Reader — `kernel_core::fdt`

Pure `no_std` reader over `&[u8]`, zero alloc, no recursion (explicit depth
counter, capped), every read bounds-checked, big-endian via
`u32::from_be_bytes`. Re-validates the header (magic, `totalsize` within the
slice, struct/strings offsets in range) — pure functions do not trust
callers, even though `bootinfo::survey` checked the magic once already.

**Closed extraction list** (growth needs an ADR, per ADR-0072 §3):

- root: `model`, `compatible`, `#address-cells`, `#size-cells`
- `/memory*` nodes: `reg` (honouring root cells; multiple ranges summed)
- `/cpus`: count of children with `device_type = "cpu"`
- `/system`: `linux,revision` (firmware-patched; absent in distributed blobs)

Not read: phandles, overlays, the memory-reservation block, `/chosen`,
`/soc`. Malformed structure → typed `Err`, reported as `unknown`, never a
panic.

### 2. Description and reconcile — `kernel_core::hwdesc`

Fixed-capacity `HwDescription` (inline model string, optional revision, up
to 4 memory ranges, cpu count) with per-fact provenance. Pure
`reconcile(&CompiledClaims, &HwDescription)` yields one verdict per fact:

| Verdict               | Memory semantics (the interesting case)                                          |
| --------------------- | -------------------------------------------------------------------------------- |
| `matches`             | discovered total ≥ compiled identity end, within the compiled map's intent       |
| `beyond compiled map` | discovered total > identity end — the evidence line for a future map-raise ADR   |
| `short`               | discovered total < identity end (mis-deploy or bad blob) — printed, not acted on |
| `unknown`             | zero-size `/memory` (un-patched distributed blob) or parse failure, with reason  |

The DTB reports ARM memory (the VideoCore share is excluded), so the
comparison is `>=`, never equality.

### 3. Report — `bootstrap::discover`

Runs immediately after the `DTB mapped` line, on the RO kernel mapping only.
One line per fact, unconditional; the no-DTB boot prints the same lines with
`unknown (no dtb)`. Shape (values illustrative):

```
discover: model "Raspberry Pi 4 Model B" rev=0xc03115 (fdt)
discover: memory 4096 MiB (1 range) — beyond compiled map (identity 2 GiB)
discover: cpus 4 (fdt) smp-seen=4 — matches
discover: display compiled=off (claim, not probed)
```

### 4. Oracle rows

Added to `scripts/lib/boot-oracle.sh` (shared: QEMU gate + HW transcript
check demand them identically). Shapes, not values — HW is 4 GB, fixtures
are un-patched — and every row accepts its `unknown (…)` form so a degraded
boot passes while a _silent_ one fails.

### 5. QEMU coverage — `-dtb` fixture

QEMU `raspi4b -kernel` passes no DTB (the machine builds no FDT), so the
gate would only ever exercise the `no dtb` path. `qemu-boot-check.sh` now
passes the pinned fixture blob with `-dtb` in the first of its two runs, so
CI parses a real tree every run; the second run stays DTB-less and covers
the degraded shape.

### 6. Fixtures

`crates/kernel-core/tests/fixtures/` with a provenance `MANIFEST.txt`
(third_party style): `bcm2711-rpi-4-b.dtb` from raspberrypi/firmware tag
`1.20250430` (the tag `third_party/blobs` already pins), sha256 recorded.
The distributed blob is the honest hard case: mixed cells
(`#address-cells=2`, `#size-cells=1`), zero-size `/memory` (firmware
patches it), no `linux,revision`. Negative fixtures are hand-built byte
arrays in the tests: truncated header, truncated struct block, `totalsize`
beyond the slice, depth bomb, `cells=0`.

Residual: a firmware-patched dump captured from the Pi 4B (real revision +
RAM) as a second positive fixture — next HW session.

## Evidence

| Line                  | Meaning                                    |
| --------------------- | ------------------------------------------ |
| `discover: model …`   | parse reached the root props               |
| `discover: memory …`  | `/memory` decoded and reconciled           |
| `discover: cpus …`    | `/cpus` counted and cross-checked vs `smp` |
| `discover: display …` | compiled claim reported                    |

Runners: host tests (fixtures, positive and hostile); QEMU `-dtb` fixture
run + DTB-less run; HW Pi 4B transcript (firmware-patched values) — stamp
residual until the next SD/serial session.

## Non-goals (residuals)

- x86 lab parity (ADR-0072 slice 2)
- Display presence probing (slice 3, amends ADR-0009)
- Consuming the discovered memory map (slice 4, supersedes part of ADR-0011)
- `/chosen`, bootargs, serial-number

## Related

- [0072](0072-hardware-self-discovery-design.md) — design and doctrine
- [0011](0011-dtb-mapped-board-constants-risk-accept.md) — compiled truth, untouched
- [0065](0065-platform-self-check.md) — the observe→decode→line→oracle mould
