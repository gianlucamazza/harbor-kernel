---
id: 0107
title: GENET bring-up moves from one-variable slices to sequence-first
status: proposed
date: 2026-08-17
related: [0001, 0096, 0105, 0106, 0108]
---

# ADR-0107: GENET bring-up moves from one-variable slices to sequence-first

## Status

**Proposed.** This ADR changes the _method_ by which the Pi 4 GENET backend is
brought up. It does not claim a NIC, does not move any status, and does not
weaken the evidence gate in [ADR-0105](0105-pi4-nic-backend-boundary.md), which
stays exactly as written.

## Context

Between 2026-08-14 and 2026-08-16 the GENET backend was advanced by
**one-variable slices**: each commit wrote one Linux register onto the boot
path, each was stamped on silicon, and each was recorded in
[`../roadmap.md`](../roadmap.md) as paid — almost always as _negative_
evidence. Twenty-five such slices are on the record. Every one of them is
honest and every one of them is still true.

The method was correct while the hypothesis space was **"one register is
missing."** Under that hypothesis a single variable per boot is the cheapest
possible experiment and the one that cannot lie about which change moved the
needle.

That hypothesis is now falsified by its own results. Twenty-five registers
later the outcome has not moved: TDMA retires the descriptor (CONS posts), the
UniMAC TSV counters (`0x49c`, `0x4a8`, `0x4ec`) stay at zero, and the host pcap
has never contained `0x88b5`. A search that has enumerated most of Linux's
`init_umac` register set without changing the outcome is not a search for a
missing register.

The [2026-08-17 excellence review](../reviews/2026-08-17-excellence.md) §A
compared the boot path against the Linux driver source line by line — not
against a reconstruction of it — and found four structural discrepancies that
**no single-variable slice could have found, because every slice added its
register at the same point in the sequence, and that point is the defect**:

| Finding | Harbor                                                                                                                                | Linux                                                                                                                                                                                                                                                     |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| F-4     | UniMAC / TBUF / RBUF programmed **after** `DMA_EN` (`src/drivers/genet.rs:355-383`)                                                   | `init_umac()` runs before `bcmgenet_init_dma()`, which enables DMA last (`bcmgenet.c:2602-2660`, `:3172-3180`)                                                                                                                                            |
| F-5     | `UMAC_TX_FLUSH` pulsed after DMA enable, with a register readback as the settle (`:547-552`); `RBUF_CTRL = 0` with no settle (`:133`) | pulsed inside `bcmgenet_init_dma` before any ring, `udelay(10)` between the writes (`bcmgenet.c:3113-3115`); `reset_umac` has `udelay(10)` (`:2562-2563`); `bcmgenet_umac_reset` pulses `RBUF_CTRL` BIT(1) with `udelay(10)` on both edges (`:3299-3311`) |
| F-6     | TX descriptor carries `DMA_OWN` and `DMA_WRAP` (`:795-807`)                                                                           | `bcmgenet_xmit` sets neither (`bcmgenet.c:2184-2200`)                                                                                                                                                                                                     |
| F-7     | HFB never touched (0 occurrences in the tree)                                                                                         | `bcmgenet_hfb_init` before DMA init (`bcmgenet.c:3380`, `:724`); `HFB_CTRL = 0` is how Linux _disables MAC receive_ (`:3438`)                                                                                                                             |

Continuing with the next single variable (`RBUF_CHK_CTRL`, currently marked
`next`) spends a silicon boot to confirm one more negative.

## Decision

### 1. The unit of experiment becomes a coherent group, not a register

A bring-up slice may now change **several registers at once** when, and only
when, the change is a **single claim about the sequence** — an ordering, a
settle contract, a block that was never touched. The slice names that claim in
its commit message and in its roadmap row.

This is not a licence to batch unrelated edits. The test is falsifiability: if
the boot's outcome changes, the row must say _which claim_ the outcome
supports, and a group whose members could each independently explain the change
is not one group.

### 2. What does not change

- **Every boot is still paid.** A slice is unpaid until a Pi 4 transcript
  exists, `make hw-check` is clean, and the roadmap row names the transcript
  and `src=`.
- **The ADR-0105 evidence gate is untouched.** Probe, link state, one bounded
  TX, one bounded RX, reset/recovery and absent-device refusal, on real
  silicon. Plus what has never yet been produced: a host capture containing
  EtherType `0x88b5` with source `02:00:00:00:00:01`, **and** a non-zero
  UniMAC TSV in the same boot.
- **No status moves without that capture.** ADR-0105 and ADR-0106 stay
  `proposed` until it exists.
- **Negative evidence is still evidence** and still gets a row. Grouping
  changes what a row claims, not whether a row is written.

### 3. The reference is the source, not the recollection

Claims about "what Linux does" in ADR-0106 and in roadmap rows must cite
`bcmgenet.c` / `bcmgenet.h` by function and line. A register value recalled
from memory is a hypothesis, not a citation, and the twenty-five slices show
the difference costs boots.

### 4. The next hypothesis class is named now, not after the next failure

If the sequence corrections land and the UniMAC TSV is **still** zero, the
remaining hypothesis class is **not** "one more register." It is:

| Class                            | Why it is next                                                                                                                            | How it would be investigated                                                                           |
| -------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Clock / power to the RGMII block | Linux `bcmgenet_open` calls `clk_prepare_enable(priv->clk)` before anything else (`bcmgenet.c:3359`); Harbor has no clock contract at all | read back what the firmware left, before and after each write; a design slice in ADR-0106, not a guess |
| Firmware-left controller state   | Pi 4 firmware initialises GENET for network boot, so nothing is at reset defaults                                                         | a boot that _only_ dumps the block's state, paid as its own row                                        |

Writing this down before the experiment is the point: it stops the failure of
the sequence hypothesis from being answered with another register.

### 5. Ordering constraint

The link fix ([ADR-0108](0108-boot-path-link-acquisition.md)) lands **before**
the sequence group. While the boot path refuses TX and RX at
`LinkState::Down`, a sequence change cannot be measured — the doorbell is never
rung, so every boot reports the same refusal regardless of what changed.

## Consequences

### Positive

- The four structural discrepancies become testable in two boots instead of
  five slices, without giving up per-boot evidence.
- ADR-0106's leftovers table stops growing one register at a time toward a
  conclusion the register set cannot reach.
- The "what if the sequence is right and it still fails" branch has an owner
  and a protocol before it is needed.

### Negative / costs

- A group that changes the outcome is less precise than a single variable
  about _which_ member did it. Mitigation: §1's falsifiability test, and the
  freedom to bisect a group across two boots when the answer matters.
- This ADR admits that twenty-five paid rows did not converge. That is the
  record working as intended, not a defect in it.

## The gate that catches its own reversal

There is no automated gate for a method. What catches a silent return to
one-variable-at-the-wrong-point is
[`../roadmap.md`](../roadmap.md): a slice that cannot state the sequence claim
its row supports has no row to write, and `make roadmap-evidence` refuses a
`done` cell with no evidence line.

The two gates that this ADR's companion work adds — `genet*` inside the
mutation scope, and a layers-table check — are the mechanical half, and they
are recorded where they belong: [ADR-0096](0096-gates-that-do-not-depend-on-remembering.md)
already owns the principle, and [ADR-0049](0049-deferred-residuals.md) already
named "the next membership miss" as the trigger that has now fired.
