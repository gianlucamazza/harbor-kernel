---
id: 0066
title: P2 — SD media persistence for the durable store (EMMC2 PIO)
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0013, 0029, 0036, 0045, 0049]
---

# ADR-0066: SD media durable store (P2 power-cycle slice)

## Acceptance status

**Accepted** (2026-08-09), on delegated authority: the owner directed this
slice ("analizza e pianifica P2", then approved the plan with the standing
delegation for its implementation ADR) with the explicit bar of modern,
self-describing best practice — no magic block addresses.

Closes the last true-media residual of **P2**
([ADR-0045](0045-p2-durable-store.md) §5: "True SD/eMMC media and
power-cycle durability on Pi (`done (HW)`)").

## Context

The durable store is a 4 KiB NOLOAD DRAM section: it survives a soft reset
because `boot.s` does not clear it, and loses everything on a power cycle
because nothing ever touches non-volatile media. The kernel has **no** SD
code; the BCM2711 EMMC2 controller (SD card slot) sits at a base already
inside the mapped Device-nGnRnE peripheral window. `reset: PowerOn` alone
cannot prove a power cycle (QEMU reports it on every reset): the honest
oracle is PowerOn **plus content evidence** — a boot counter written by the
previous boot and read back before any write.

## Decision

### 1. SDHCI scope — PIO, 1-bit, single-block, SDHC/SDXC only

| Item     | Choice                                                                                                                                                                 |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Transfer | CMD17/CMD24 single-block PIO through the buffer port; no DMA (no PoC cache maintenance or DMA-safe allocator exists — a separate arch slice)                           |
| Bus      | 1-bit. A full flush is 18 sectors ≈ 9 KiB; width buys nothing and doubles the init machine                                                                             |
| Cards    | SDHC/SDXC (ACMD41 HCS, CCS=1, block addressing). SDSC → honest `unsupported`: dropping byte/block duality removes a class of off-by-512 bugs                           |
| Clock    | Init ≤400 kHz, data ≤25 MHz, divider fixed conservatively for a 200 MHz base clock (no mailbox in this slice; a 100 MHz real base just halves a 7 ms transfer)         |
| Init     | Pure state machine in `kernel_core::sdcard` (states as data, host-tested: golden SDHC walk, SDSC/legacy rejection, ACMD41 bound); the driver only executes transitions |

### 2. Placement — dedicated MBR partition, discovered by type

A **1 MiB partition of MBR type `0x7f`** (the type designated for
experimental use), created once by a host script in the card's unpartitioned
tail. The kernel reads sector 0, parses the four MBR entries
(`kernel_core::mbr`, pure, host-tested) and takes the `0x7f` entry's
LBA+length as the store window.

- The card's own partition table **enumerates** the store — the same shape
  as the project's authority rule, and it survives repartitioning as long as
  the entry exists. No magic LBA constant to collide with anything.
- No `0x7f` entry → `durable-media: no-partition`, boot proceeds degraded
  (DRAM-only store, today's behavior). GPT protective MBR (`0xEE`) →
  fail-closed, documented.
- The host verifies the same invariant independently (`sfdisk`-based guard
  at deploy).

### 3. Media format — DURB v1 payload, A/B slots, header commits last

Inside the partition: **slot A** and **slot B**, each 1 header sector +
8 payload sectors (the 4 KiB [ADR-0045](0045-p2-durable-store.md) DURB
block byte-for-byte). Header (`DMH1`): version, u64 sequence, CRC32 of the
payload.

- **Flush:** seq+1 → write the _other_ slot's payload sectors → write its
  header **last** (the commit point).
- **Load:** validate both slots (magic + CRC); highest valid seq wins; none
  valid → fresh (matches DURB bad-magic-is-empty semantics).
- A torn header or payload loses at most the newest flush, never the last
  good state. DURB itself is unchanged — atomicity is a media concern and a
  payload-internal CRC could not arbitrate two slots.
- **Boot counter:** reserved key `boot` (u32) inside the store, incremented
  by bootstrap each boot — it rides the existing put/get path, which is what
  makes it honest content evidence.

### 4. Boot flow and layering

`bootstrap` (which may import every layer) orchestrates; `durable` keeps its
arch-only import and gains `snapshot`/`restore` byte access; the driver
(arch+irq only) never knows the store exists. No new layering edge.

Sequence, before the existing `durable: reloaded` demo: probe EMMC2
(fault-recoverable, rng200 pattern; absent → `durable-media: absent
(NotPresent)`, boot proceeds) → read MBR, find partition → `media_load` →
`durable::restore` → read key `boot` (**before any put** — evidence of the
previous boot) → print the load-evidence line → put `boot = N+1` → existing
demo unchanged → `durable::snapshot` → `media_flush` → read-back verify.
One explicit flush point per boot; no flush-on-put (would need a layering
edge and unbounded write amplification).

## Evidence

| Check                                                                                                                                                                                             | Gate                                                                                                                                                                                             |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Pure layers (SDHCI encodings, init machine, media wrapper, MBR)                                                                                                                                   | kernel-core host tests                                                                                                                                                                           |
| Healthy trio on one boot: `durable-media: boot=<N> from=Fresh\|Previous part=0x7f slot=A\|B seq=<M>` + `durable-media: flushed` + `durable-media: verified` — or exactly one honest degraded line | boot-oracle (both runners); `DURABLE_MEDIA_EXPECT` pins the mode                                                                                                                                 |
| Emulated power cycle: second QEMU boot on the same scratch card image — counter increments, slot alternates                                                                                       | qemu-boot-check two-boot phase                                                                                                                                                                   |
| Absent path stays honest                                                                                                                                                                          | qemu-boot-check no-card phase (silicon always has a card; only QEMU can regression-test this)                                                                                                    |
| True power cycle                                                                                                                                                                                  | HW protocol: boot 1 (`from=Fresh boot=1`) → physical power unplug ≥5 s → boot 2 (`reset: PowerOn`, `from=Previous boot=2`) → host `durable-read.sh` confirms the bytes on the card independently |

The HW transcript is one boot's log: `hw-transcript-check` requires
`from=Previous` with `boot>=2`, so the canonical transcript is recorded on a
second-or-later powered boot.

## Non-goals (this slice)

DMA/ADMA2 (needs PoC cache ops), 4-bit bus, UHS/high-speed, multi-block
CMD18/CMD25, any filesystem (FAT paths stay withdrawn per
[ADR-0029](0029-agent-store-in-image.md)), GPT beyond fail-closed, wear
management, eMMC devices, **EL0 storage capabilities** (the remaining P2
residual, a separate slice).

## Risks (accepted)

- **EMMC2 vs Arasan routing**: the SD slot is EMMC2 by default on BCM2711;
  a probe/init failure on silicon suspects routing/power domain first — the
  degraded oracle line makes it visible, not fatal.
- **QEMU sdhci fidelity**: permissive model; QEMU proves logic, silicon
  proves media. Poll bounds sized for silicon.
- **Header-sector atomicity** is assumed card-internal; the A/B scheme makes
  a violation a degradation (lose the newest flush), not corruption.
