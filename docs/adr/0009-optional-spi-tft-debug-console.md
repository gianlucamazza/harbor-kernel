---
id: 0009
title: Optional SPI TFT debug console (Waveshare 3.5″ / ILI9486)
status: proposed
date: 2026-08-05
---

# ADR-0009: Optional SPI TFT debug console

## Acceptance status

**Proposed.** No running code is constrained yet. Accepting this ADR (or a
refined successor) is the gate before SPI0 / ILI9486 land in `src/`. Hardware
tables for the HAT live in [`../hardware.md`](../hardware.md); this record is
the **policy** only.

## Context

Harbor’s only human I/O today is the PL011 serial console. That remains correct
for QEMU, CI (`boot-check`), and headless lab work. A common lab accessory is a
**3.5″ SPI TFT HAT** (Waveshare 3.5inch RPi LCD A/B/C and pin-compatible clones):
480×320, **ILI9486**, resistive touch **XPT2046**, stacked on the 40-pin header.

That HAT is useful when:

- the header is occupied and probing UART pads is awkward;
- bring-up or panic needs a glanceable status without a serial session;
- demos want something on the board that is not a blank slab.

It is **not** required for the agent roadmap (M4 IPC → M6 driver-as-agent). It
does not replace UART. It is not HDMI/DSI/mailbox GPU work.

The tree has no SPI driver, no generic GPIO beyond UART pinmux, and no
framebuffer path. Adding the panel therefore implies new `drivers/` and
`bsp/rpi4` surface, which must stay inside the existing layering rules
([`../architecture.md`](../architecture.md)).

## Decision

### Role

1. **UART remains the primary console.** All gates that exist today stay
   UART-based. QEMU and `make check` do not depend on a display.
2. The TFT is an **optional debug/status sink**: boot banner, coarse status
   (ticks, heap), panic banner. Not a general graphics stack, not a compositor,
   not a userspace framebuffer API.
3. The feature is **compile-time optional** (cargo feature, working name
   `display-waveshare35`, default **off**). Images without the feature must
   match today’s behaviour.
4. **Boot probe with silent fallback:** when the feature is on, init runs
   *after* the first UART lines. Failure to init logs once on serial and leaves
   the machine headless; it does not refuse boot.
5. **IRQ handlers never write the panel** (same rule as UART TX —
   [ADR-0008](0008-irq-handler-policy.md) / architecture rule 6). Only the
   voluntary path (bootstrap, idle, tasks, panic steal path) may paint.
6. **Touch is out of scope for v1.** Phase 2 may add XPT2046 on the same SPI
   bus; it needs its own design note, not a silent expansion of this ADR.
7. **Layering:**
   - `drivers/spi`, `drivers/ili9486` (or equivalent) are board-agnostic;
   - pinmux, bases, CS/DC/RST numbers, SKU constants live in `bsp/rpi4`;
   - console/bootstrap policy decides *what* is mirrored, not how SPI works.
8. **Performance policy for v1:** cell/glyph dirty updates via ILI9486 window
   commands, not full 300 KiB frame blits on every log line. SPI stays **polled**
   until a measured need for DMA/IRQ is written down.
9. **Verification:** “display works” is a **hardware** claim only (HAT + Pi 4B),
   recorded in [`../verification.md`](../verification.md) the same way other
   silicon rows are. QEMU does not close the claim. `make check` with the
   feature off must stay green; host unit tests may cover pure maths (divisor
   clamps, cell layout) in `kernel-core` if extracted.

### Explicit non-goals (this ADR)

- VideoCore mailbox framebuffer, HDMI, DSI, DPI
- Linux fbtft / dtoverlay dependency
- Driver-as-agent (M6) packaging of the panel
- Guaranteeing A/B/C init sequences are identical without SKU tables
- Softfloat exceptions: no third-party crates that pull FP/SIMD

## Consequences

### Positive

- Lab observability without inventing a GPU stack or derailing M4–M6.
- Feature flag keeps CI and default images lean and HAT-free.
- Documented pinout and layering reduce the chance of a one-off `bsp` spaghetti
  driver.

### Negative / costs

- New MMIO surface (SPI0) and GPIO ownership discipline beside UART.
- SKU variance (A/B/C, clones) can produce white-screen failures until the init
  table matches the PCB in hand.
- Full-frame SPI is slow; text-console discipline is mandatory or the idle path
  will feel wedged.
- Panic path complexity: optional second sink beside UART `steal`.

### Gate that would catch a reversal

| Reversal | How we notice |
|----------|----------------|
| Display becomes required for boot | `make boot-check` / default image fails without HAT; or boot refuses when probe fails |
| IRQ paints the panel | Layering / review vs architecture rule 6; hang or re-entrancy under RX flood |
| Driver hard-codes Pi pins inside `drivers/` | `make layering` |
| “Done” claimed from QEMU only | Missing silicon row in `verification.md` |
| Touch or GPU scope sneaks into v1 | Diff grows past SPI + ILI + text cells without a successor ADR |

## Alternatives considered

| Alternative | Reason not selected |
|-------------|---------------------|
| HDMI via VC mailbox | Wrong hardware for this HAT; larger firmware contract |
| Bitbang SPI only | Acceptable spike, but BCM SPI0 is the durable path on Pi 4 |
| Always-on display in default image | Breaks HAT-free and QEMU-primary workflow |
| Full RGB framebuffer UI in v1 | Scope creep; 300 KiB blits fight the idle/WFI console model |
| Defer entirely until M6 driver-as-agent | Valid sequencing, but loses early lab value; this ADR keeps it a side-track that must not block M4 |

## Related

- [`../hardware.md`](../hardware.md) — pinout and SKU notes
- [0006](0006-cooperative-execution-model.md) — voluntary path only for heavy work
- [0008](0008-irq-handler-policy.md) — IRQ handlers stay short and non-transmitting
- [`../architecture.md`](../architecture.md) — layering rules 1–6
