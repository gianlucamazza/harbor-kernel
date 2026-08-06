---
id: 0011
title: DTB mapped but board truth stays compiled-in (F15 risk-accept)
status: accepted
date: 2026-08-05
accepted: 2026-08-05
---

# ADR-0011: DTB mapped, board constants remain the source of truth

## Acceptance status

**Accepted.** Records finding **F15** from the 2026-08-04 multi-role review as
an explicit risk-accept for the current single-board lab (Raspberry Pi 4 Model
B). This is not a parse implementation and not a claim that multi-board is
supported.

## Context

Harbor captures the firmware DTB pointer at boot, validates the FDT magic when
present, and maps the blob **read-only** into the kernel map. It does **not**
parse nodes for MMIO bases, IRQ numbers, or clocks. Those values live in
`bsp/rpi4/memmap` and related BSP modules.

F15: *“DTB mapped but not parsed; board truth is hard-coded.”* Leaving that
state without a decision is dishonest: either parse is the roadmap, or the
hard-coded path is accepted with a clear boundary.

## Decision

1. **Source of truth for Pi 4B lab:** compiled BSP constants (PL011, GIC, SPI0,
   RNG200, pin numbers, assumed clocks with `config.txt` pins such as
   `enable_uart=1`, `enable_gic=1`, `core_freq_min=500`).
2. **DTB role today:** presence/absence and size for boot diagnostics; mapped
   RO so a future parser can consume it without a second identity-map pass.
3. **Out of scope until multi-board or a named milestone:** full FDT walk,
   overlay application, or deriving SPI/UART bases from the tree.
4. **QEMU:** no firmware DTB is expected; `x0` may be zero — already soft-handled
   (`no DTB` line). Board constants still apply to the QEMU machine model.

## Consequences

### Positive

- Single board path stays simple and reviewable.
- No half-parser that silently disagrees with `memmap`.
- F15 is closed as a *decision*, not left as ambient debt.

### Costs / residual risk

- Porting to another BCM or Pi model requires BSP edits, not “load a new DTB”.
- If firmware layout diverges from assumptions, silicon shows wrong clocks or
  missing devices — mitigated by serial diagnostics and verification transcripts,
  not by guessing from an unparsed tree.

### Gate that would catch a reversal

| Reversal | Signal |
| -------- | ------ |
| Silent dual source (some bases from DTB, some from memmap) without ADR | Layering / review: two truths |
| Claim “DTB-driven board support” in README while constants rule | `doc-claims` / review |
| Parser that ignores `memmap` without superseding this ADR | New ADR required |

## Alternatives considered

| Alternative | Why not now |
| ----------- | ----------- |
| Full FDT parse for all peripherals | Large surface; wrong cost before M4 |
| Drop mapping the DTB entirely | Throws away a clean future hook already paid for |
| Feature-gate board constants | Constants are the product for this SKU |

## When to revisit

- Second supported board or SKU that cannot share `bsp/rpi4`.
- M6 driver-as-agent needs topology that firmware encodes and we do not.
- Explicit product requirement: “boot same image on Pi 3/5 with DTB only”.

## Related

- [architecture.md](../architecture.md) — F15 row; open findings
- [`../hardware.md`](../hardware.md) — Pi 4B assumptions
- `src/arch/aarch64/bootinfo.rs` — DTB pointer / magic
- Multi-role review 2026-08-04 — F15
