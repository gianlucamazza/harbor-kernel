---
id: 0072
title: Hardware self-discovery as boot evidence — verify, don't select
status: accepted
date: 2026-08-09
accepted: 2026-08-09
related: [0009, 0011, 0015, 0065, 0073]
---

# ADR-0072: Hardware self-discovery as boot evidence (design)

## Acceptance status

**Accepted as design** (2026-08-09, on delegation from Gianluca). First code
slice: [ADR-0073](0073-discovery-first-slice-fdt-report.md).

## Context

The tree holds three discovery idioms that grew separately: the
fault-recoverable MMIO write probe (`arch::probe`, used by RNG200 and SDHCI),
the declarative platform self-check (ADR-0065: observe → pure decode → one
boot line → oracle assertion), and compile-time selection (`board-*`,
`debug-display`). Meanwhile the device tree is captured (`__dtb_ptr`),
validated, and mapped RO — and never parsed; ADR-0011 parks the parser
"until multi-board or a named milestone". This ADR is that named milestone,
scoped tightly.

The trigger is practical: facts the hardware states — board revision, RAM
size, core count, which optional peripherals answered — exist nowhere in the
boot evidence, so a transcript cannot say _which_ physical board produced it,
a 4 GB board silently runs with a 2 GiB identity map, and a headless image on
a HAT-equipped board fails as an unexplained white screen.

## Decision

### 1. Doctrine — verify, don't select

Discovery produces **evidence, never configuration**:

- Compiled BSP truth remains the only authority (ADR-0011 §1 untouched).
  Nothing dispatches, sizes, or maps based on a discovered value.
- The model is ADR-0065's, generalised: observe → pure decode (host-tested in
  `kernel_core`) → reconcile against the compiled expectation → **one
  unconditional `discover:` line per fact**. Unknown is a printed outcome,
  never an absence; silence fails the boot oracle.
- **Fail-open, standalone.** Discovery never gates the boot and never feeds
  the mechanisms it verifies. A malformed blob prints its reason and the boot
  continues; a mismatch is a printed verdict, not a refusal.
- Runtime board/SKU selection stays wrong (ADR-0015 reaffirmed). This ADR
  adds a reporter, not a chooser.

### 2. Facts and their consumers (closed inventory)

Every fact names a real consumer; anything without one is out.

| Fact                   | Source (provenance)                                         | Consumer                                                                                                                                    |
| ---------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Board model + revision | FDT `/model`, `/system` `linux,revision` (firmware-patched) | Self-describing HW transcripts; deploy diagnostics                                                                                          |
| Memory ranges          | FDT `/memory` `reg`                                         | Verify vs `IDENTITY_RAM_END`; a `beyond compiled map` verdict observed on silicon is the evidence a future identity-map-raise ADR must cite |
| CPU count              | FDT `/cpus`                                                 | Cross-check vs `smp` observation; sizing input for K8 per-core queues (ADR-0048)                                                            |
| Display/HAT presence   | Compiled (`debug-display`)                                  | Reported as compiled claim; see §4                                                                                                          |

Explicitly dropped: `/soc` ranges, clocks, IRQ routing (ADR-0011 compiled
truth rules — a half-parser disagreeing with `memmap` is the exact failure
0011 names), confidence scores, any generic fact-registry framework
(speculative surface, per ADR-0065's own rejection).

### 3. Shape

- `kernel_core::fdt` — pure, zero-alloc, bounds-checked reader over
  `&[u8]`, host-tested with fixture blobs. **Closed property list** (root
  `model`/`compatible`/cells, `/memory*` `reg`, `/cpus` cpu count, `/system`
  `linux,revision`); growth requires an ADR.
- `kernel_core::hwdesc` — fixed-capacity `HwDescription` + pure
  `reconcile(claims, observed)`; each fact carries a provenance
  (compiled / id-register / firmware-table / probe) and yields a verdict
  (matches / exceeds / differs / unknown). Board-free: addresses arrive as
  arguments.
- `arch::bootinfo` gains the consume half of its contract row
  ("early map + optional consume"): a safe accessor for the RO-mapped blob
  slice on AArch64; `None` on x86 (PVH — no DTB). No new facade module.
- `bootstrap::discover` (submodule) runs immediately after the DTB RO map,
  builds the compiled claims from `bsp::board`, prints the `discover:` lines.
  No new top-level module; layering untouched.

The DTB is read **only** through the RO kernel mapping, after `mmu::map` of
the blob — never through the early map (`bootinfo`'s "validated once" rule
stands).

### 4. Display: the honest refusal

Runtime detection of the SPI TFT is **not** in this design's first slices:
the panel path is write-only (SPI has no addressed ack; a write probe cannot
distinguish absent from present), RDDID read-back is unreliable across
Waveshare-class clones, and the spec-compliant path (HAT ID EEPROM on i2c-0)
requires a new driver and is unpopulated on those clones anyway. Discovery
reports the _compiled_ claim (`debug-display` on/off) so the transcript at
least states which image was deployed — ending the silent half of the
white-screen failure. Actual presence detection is a later slice behind an
ADR-0009 amendment, and per ADR-0009's reversal table it must never gate
boot. "No runtime multi-SKU guessing" stands.

### 5. Relation to ADR-0011 (companion, not supersession)

ADR-0011 §3 parks the FDT walk "until multi-board or a named milestone" and
its reversal table demands a new ADR for any parser. This is that ADR, with
the narrowest possible grant: **parse for verification reporting only**.
Deriving any base address, clock, or map bound from the DTB remains
forbidden; consuming the discovered memory map (raising the identity map)
requires a further ADR that supersedes the relevant clause of 0011 and must
cite discovery evidence observed on silicon.

### 6. Slices

1. [ADR-0073](0073-discovery-first-slice-fdt-report.md): FDT reader +
   `discover:` report on AArch64 (this slice).
2. x86 lab parity: same grammar from CPUID/PVH in the L0 lab image.
3. Display presence (own ADR, amends 0009).
4. Identity-map raise (own ADR, partially supersedes 0011, cites HW
   evidence).

## Consequences

### Positive

- HW transcripts become self-describing (which board, how much RAM, how many
  cores) at zero authority cost.
- The 2 GiB identity-map clamp stops being invisible: a 4 GB board prints
  the excess every boot.
- One doctrine for future discovery instead of a fourth ad-hoc idiom.

### Negative / residual

- The reader is a parser in the TCB, mitigated by: pure code, total
  bounds-checking, host fixtures including hostile inputs, no alloc.
- The distributed (un-patched) DTB reports zero-size memory; only
  firmware-patched blobs carry real values — the report must print that
  state honestly (`unknown (zero-size)`), and QEMU coverage uses a fixture
  blob passed with `-dtb`.

### Gates

| Reversal                                                 | Catch                                                                      |
| -------------------------------------------------------- | -------------------------------------------------------------------------- |
| A discovered value configures anything (map, base, size) | Review + this ADR's non-goal; layering unchanged means no new import paths |
| `discover:` line missing from a boot                     | boot oracle (shared: QEMU + HW transcript)                                 |
| Property list grows without ADR                          | Review; the list is enumerated in ADR-0073                                 |
| Discovery failure blocks boot                            | By construction fail-open; oracle accepts `unknown` shapes                 |

## Related

- [0011](0011-dtb-mapped-board-constants-risk-accept.md) — compiled truth; this ADR is the parser milestone it names
- [0015](0015-multi-arch-scaffold.md) — no runtime selection, reaffirmed
- [0065](0065-platform-self-check.md) — the doctrine this generalises
- [0009](0009-optional-spi-tft-debug-console.md) — display stays compile-time until amended
- [0073](0073-discovery-first-slice-fdt-report.md) — first code slice
