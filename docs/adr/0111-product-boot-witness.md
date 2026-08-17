---
id: 0111
title: The product leaves a witness that it booted, off the serial line
status: proposed
date: 2026-08-17
related: [0045, 0066, 0087, 0096, 0098]
---

# ADR-0111: A boot witness the host can read without the UART

## Status

**Proposed** (2026-08-17). Answers issue #89, filed from the 2026-08-17
hardware session.

## Context

For an afternoon the hardware loop could not tell **"the board did not boot"**
from **"the observation channel is dead"**. The two produce the same artifact:
an empty capture.

The cost was not hypothetical. A CP2104 adapter dropped off the USB bus four
times in an hour, each time leaving a zero-byte log. Then a real regression — a
deploy that removed the board's device tree without writing one — produced the
identical symptom. With one channel and no cross-check, the search went through
a dead adapter, a suspected corrupt card, a suspected brown-out and a suspected
dead board before arriving at a missing file. Three of those four hypotheses
were mine, and every one of them was reachable only because there was nothing
to falsify them with.

### The oracle already has the witness the product lacks

`src/bootstrap/demos.rs` initialises SDHCI, finds the type `0x7f` partition,
loads the winning A/B slot, advances a `boot` counter and commits it
([ADR-0066](0066-sd-media-durable-store.md)). `scripts/host/durable-read.sh`
reads that partition with `dd` and a CRC check, **with nothing of the kernel in
the loop** — which is exactly what an independent witness means.

`demos.rs` is oracle scaffolding. `make product-builds` strips it: _83 items
unreachable without the oracle_. So the image that ships has no witness at all,
and the image that has one is not the image under test.

That gap bit twice on the same day. A measurement was designed on the
assumption that the product wrote the store; the number could not move; and
"the board is not booting" was asserted as fact from a number that was never
going to change.

## Decision

### 1. The product advances a boot counter on media, once, early

The product boot path gains the same sequence the oracle runs — SDHCI init,
MBR parse, `media_load`, `durable::put(b"boot", …)`, commit — placed after
`report_reset` and **before** the loader runs any agent.

Early, because the point is to witness a boot that later fails. A witness
written at the end of bring-up cannot witness anything that stops bring-up, and
that class is precisely what the afternoon was spent on.

### 2. One write, and what it therefore does not prove

It proves the board reached the witness point: firmware handed off, the image
was found and entered, the MMU came up, reset was classified, and the SD stack
answered. It does **not** prove bring-up completed. `make hw-check` over a
serial transcript is what judges a boot as a whole, and this does not replace
it.

A second commit at the end would separate _arrived_ from _completed_, and is
refused here: it doubles the per-boot write for a distinction the serial oracle
already draws whenever the serial line is alive, and the case this exists for is
the one where it is not.

So the honest reading of the counter is **"the previous boot got this far"**,
and that is the reading `durable-read.sh` output should be given.

### 3. Degradation is a line, never a refusal

No card, no `0x7f` partition, an unreadable MBR, a failed load: each prints one
honest line and the boot continues on the DRAM-only store, exactly as
[ADR-0045](0045-p2-durable-store.md) already behaves. A witness that can stop
the product from booting is worse than no witness, because it converts an
observation aid into a failure mode.

### 4. One implementation, not a second

The product path calls the same `kernel_core::durable_media` A/B slot
discipline the oracle does. The store the P2 hardware claims rest on is the
store this writes, so a second implementation of the commit would put those
claims at risk of a divergence nothing compares — the failure
[ADR-0110](0110-a-model-is-consumed-or-declared.md) has just finished paying
for one directory over.

## Consequences

### Positive

- An empty capture stops being ambiguous: `durable-read.sh` says whether the
  board ran.
- The product image gains the property its own evidence assumed it had.
- Every hardware session gets a second channel that does not pass through a
  USB-serial adapter, which is the component that failed four times in an hour.

### Negative / costs

- **One 512-byte SD write per boot**, alternating slots. Not free, and not a
  concern at development boot rates against a card rated in thousands of
  program/erase cycles per block — but it is a write the product did not make
  before, and a product that boots in a loop would make it in a loop.
- **The product image grows by 8192 B** — 151 696 → 159 888, measured, and the
  count of items unreachable without the oracle falls from 83 to 67. SDHCI, MBR
  parsing and the media wrapper are now in what ships.
- **The witness point is not the boot's end**, so a counter that advanced is
  not a boot that worked. Stated above; worth restating wherever the number is
  read.
- The product now writes the region P2's durable claims are made against.
  Mitigated by sharing the implementation rather than copying it, not
  eliminated.

## Alternatives rejected

| Alternative                               | Why not                                                                                                                      |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| Blink the ACT LED in a pattern            | Needs a human watching at the right moment, and cannot be read after the fact. The afternoon's evidence was read hours later |
| A second serial adapter                   | Doubles the thing that broke, and the failure was the USB bus dropping the device, not the cable                             |
| Write only on failure                     | The failure this witnesses is a boot that stops before it can know it failed                                                 |
| Keep it in the oracle image and test that | Then the image under test is not the image that ships, which is how the assumption behind the wrong measurement got in       |
| Commit twice (arrival + completion)       | Doubles the write for a distinction the serial oracle already makes when it can, and this exists for when it cannot          |

### Where the two images still differ

The product commits at open. The **oracle** commits later, from
`demos::run_all`, because its `cfg` and blob demos put keys after the boot path
has run and ADR-0066 keeps one flush point per boot. So the two images differ in
_when_ the single write happens, not in _whether_ the witness is in the image —
which was the actual defect. `make product-boot-check` boots the product, so the
gate below judges the early commit, not the oracle's.

## The gate that catches its own reversal

`make product-boot-check` asserts the product's own output, and gains the
`durable-media: boot=` line: an image that drops the witness fails the gate
rather than quietly shipping without it. The line is part of the product
oracle's composition minimum, not a debug print.

On hardware, `make hw-check` over a transcript and `scripts/host/durable-read.sh`
over the card are then two accounts of one boot — the shape
`make hw-store-audit` already uses for the agent store.

## References

- [ADR-0066](0066-sd-media-durable-store.md) — the A/B slot discipline this uses
- [ADR-0045](0045-p2-durable-store.md) — degraded paths are lines, not refusals
- Issue #89 / [2026-08-17 review](../reviews/2026-08-17-excellence.md),
  postscript 2
