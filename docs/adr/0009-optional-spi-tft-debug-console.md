---
id: 0009
title: Optional SPI TFT status surface (ILI9486 / Waveshare-class HAT)
status: accepted
date: 2026-08-05
accepted: 2026-08-05
---

# ADR-0009: Optional SPI TFT status surface

## Acceptance status

**Accepted** (2026-08-05). Policy is closed: implementation must follow the
Decision and the anti-patterns list. Pin tables live in
[`../hardware.md`](../hardware.md). First code under this ADR is the reusable
foundation (contracts, GPIO, timer delay, then SPI0) — not a vendor monolith.

## Context

Harbor’s only human I/O today is the PL011 serial console. That remains correct
for QEMU, CI (`boot-check`), and headless work. A common lab accessory is a
**3.5″ SPI TFT HAT** (Waveshare 3.5inch RPi LCD A/B/C and pin-compatible clones):
480×320, controller **ILI9486**, resistive touch **XPT2046**, on the 40-pin
header.

Useful when UART is awkward to attach and a glanceable **status** surface helps
bring-up or panic triage. It is **not** HDMI/DSI/mailbox GPU work, not a
general graphics stack, and not an M4–M6 milestone.

The tree has no SPI driver, no general GPIO beyond UART pinmux, and no panel
path. A careless integration (monolithic “Waveshare driver”, bitbang, full-frame
blits, silent probe, CS owned by the panel driver) would force a rewrite when
touch or a second SPI slave appears. This ADR forbids that class of shortcut.

## Decision

### Role

1. **UART remains the primary console.** Gates stay UART-based. QEMU and
   `make check` do not depend on a panel.
2. The TFT is an **optional structured status surface** (banner, coarse runtime
   slots, panic banner) — **not** a mirror of every serial log line.
3. Product enablement is **compile-time optional** (cargo feature, working name
   `debug-display`, default **off**). Default images match today’s behaviour.
4. Waveshare (or any SKU) is a **BSP product profile**, not the name of the
   driver stack. Core modules are generic (`spi`, `ili9486`, text grid).

### Layering (no monolithic display driver)

```text
console / bootstrap / panic     status policy (what to show)
        │
text grid / glyph               pure layout + raster (host-testable)
        │
drivers/ili9486                 panel protocol; declarative init table
        │  SpiDevice + DC/RST pins
drivers/spi  (+ local traits)   SpiBus controller; SpiDevice = CS-scoped txn
        │
bsp/rpi4                        bases, pinmux, SKU profile (e.g. Waveshare A)
```

Rules 1–2 of [`../architecture.md`](../architecture.md) hold: drivers never
name the board; the BSP does not implement ILI command sequences.
`make layering` must list any new modules in the same change that introduces
them.

### Contracts (embedded-hal 1.0 *shape*, not the crate)

Adopt the **SpiBus / SpiDevice** split standardised by embedded-hal 1.0:

| Contract | Responsibility |
| -------- | -------------- |
| `SpiBus` | Controller: mode, clock, raw transfer |
| `SpiDevice` | One slave: assert CS → transfer → deassert CS |
| `OutputPin` | DC, RST (and later touch IRQ as input) |
| Delay | Wall-time delay for panel power sequencing |

Implement these as **local traits** in the Harbor tree. Do **not** depend on
the `embedded-hal` crate in the kernel: Harbor’s layering, softfloat, and
verification gates are not the MCU app ecosystem. Document the alignment so
future readers recognise the standard shape.

**Hard rule:** the ILI9486 driver must **not** take a raw CS pin and toggle it
itself. CS belongs to `SpiDevice`. Hand-rolled CS makes the bus unshareable and
forces a rewrite for XPT2046 (phase 2).

### Panel bring-up

- Init is a **declarative** table: command / data / delay-ms operations.
- **Datasheet ILI9486** is the source of truth for opcodes and timing floors.
- Linux fbtft, Waveshare docs, and TFT_eSPI are **cross-checks** for SKU
  quirks, not blobs to paste uncommented.
- Orientation / BGR / MADCTL live in a `PanelConfig` (or equivalent), not
  scattered constants.
- SKU A / B / C are **named BSP profiles**. No runtime multi-SKU guessing in
  v1. Lab default: profile for Waveshare **A** until another PCB is confirmed.

### Timing

- Panel multi-millisecond waits use **arch timer** (`CNTPCT`) delays.
- `spin_cycles` remains only for short pad/settle spins — not panel sleep-out.

### Status surface (not serial mirror)

- Fixed cell grid (e.g. 8×16 font → 60×20 cells).
- **Dirty cell / window** updates via ILI address window + RAM write.
- **No full-screen RGB565 framebuffer** as a v1 requirement (300 KiB full blits
  are the wrong tool for status text).
- Structured slots only in v1; a rate-limited log sink would need an explicit
  policy extension, not default coupling to `println`.

### Init failure

- `init → Result<_, E>` with a meaningful error path.
- On failure: **one clear UART line** (and optional counter); boot continues
  headless. Optional accessory ≠ silent failure.
- No `unwrap` on the production boot path for missing glass.

### IRQ policy

IRQ handlers **never** paint the panel (same class of rule as UART TX —
[ADR-0008](0008-irq-handler-policy.md) / architecture rule 6). Only the
voluntary path (bootstrap, idle, tasks, panic path) may.

### SPI implementation honesty

- v1 controller is **polled** with **named spin bounds** (same discipline as
  PL011 TX busy limits). A wedged SPI must not hang panic diagnostics forever.
- The `SpiBus` trait must not encode “polling forever” so that DMA/IRQ later
  requires a different consumer API. DMA/IRQ SPI is a **separate** decision when
  measured, not a TODO inside the panel driver.

### Touch (phase 2 only)

Out of scope for the first implementation. Requires a second `SpiDevice` on the
same bus (TP_CS). No touch code until the bus-sharing shape is real.

### Feature and verification

| Gate | Rule |
| ---- | ---- |
| Default build | Feature off; behaviour identical to today |
| Host tests | SPI clock divisor maths, RGB565 packing, cell layout, `InitOp` interpreter, MADCTL fields in `kernel-core` where pure |
| Layering | Enforced on every new import edge |
| “Works” claim | **Hardware only** (Pi 4B + HAT) in [`../verification.md`](../verification.md); QEMU does not close it |
| Softfloat | No FP/SIMD; no third-party crates that violate ADR-0002 |

### Explicit non-goals

- VideoCore mailbox framebuffer, HDMI, DSI, DPI
- Linux fbtft / dtoverlay as a runtime dependency
- Bitbang SPI as a temporary path
- Monolithic vendor-named driver module in `drivers/`
- Automatic mirror of the full serial log stream
- Full-frame compositor / embedded-graphics stack in v1
- Driver-as-agent packaging (M6) — that reuses these drivers later
- Treating A/B/C as drop-in without per-profile tables

### Anti-patterns (reject in review)

1. Bitbang SPI “to see a pixel”.
2. One file that pinmuxes, runs SPI, speaks ILI, draws fonts, and hooks `println`.
3. CS toggled inside the panel driver.
4. Undocumented magic init arrays.
5. Core types named only after one vendor SKU.
6. Full-frame buffer + blit for status text.
7. Cycle-spin for multi-ms panel delays.
8. Silent probe failure.
9. Painting from IRQ.
10. “Done” without a silicon row in verification.
11. `// temporary` / `// hack` / `// TODO rewrite` left as design.

## Consequences

### Positive

- SPI and GPIO foundations are reusable (touch, sensors, flash).
- Status observability without a GPU stack or derailing M4–M6.
- Feature flag keeps CI and default images HAT-free.
- Datasheet-first init and `SpiDevice` avoid a forced rewrite at phase 2.

### Costs

- More modules and ADRs before the first pixel.
- SKU variance still needs a correct profile on real glass.
- Status surface is coarser than a full log UI by design.

### Gate that would catch a reversal

| Reversal | How we notice |
| -------- | ------------- |
| Panel required for boot | Default/`boot-check` fails without HAT; or boot refuses on probe fail |
| IRQ paints | Architecture rule 6 / review; re-entrancy under RX flood |
| Driver hard-codes Pi pins | `make layering` |
| CS owned by ILI driver | Second slave cannot share the bus without rewrite |
| “Done” from compile/QEMU only | Missing silicon row in `verification.md` |
| Serial mirror by default | SPI latency / idle path review; policy drift from this ADR |
| Vendor monolith in `drivers/` | Module layout review vs this Decision |

## Alternatives considered

| Alternative | Reason not selected |
| ----------- | ------------------- |
| HDMI / VC mailbox | Wrong hardware for this HAT; larger firmware contract |
| Depend on `embedded-hal` in-kernel | Right *contracts*, wrong dependency surface for Harbor gates |
| Bitbang then “real SPI later” | Guaranteed throwaway work |
| Full RGB framebuffer UI in v1 | Wrong cost model for status; scope creep |
| Always-on panel in default image | Breaks HAT-free and QEMU-primary workflow |
| Defer until M6 driver-as-agent | Valid sequencing, but loses lab value; this ADR keeps a disciplined side-track |
| Silent optional probe | Hides faults; optional ≠ invisible |

## Implementation order (when accepted)

1. Local contracts + general GPIO + timer-based delay.
2. SPI0 polled controller (`SpiBus` + `SpiDevice`).
3. ILI9486 + Waveshare A profile (declarative init, fill proof on silicon).
4. Status surface + panic banner behind `debug-display`.
5. Touch only after (2) is real — separate note/ADR if input policy changes.

## Related

- [`../hardware.md`](../hardware.md) — pinout and SKU notes
- [0002](0002-softfloat-kernel.md) — no FP/SIMD in the kernel
- [0006](0006-cooperative-execution-model.md) — heavy work on the voluntary path
- [0008](0008-irq-handler-policy.md) — IRQ handlers stay short and non-transmitting
- [`../architecture.md`](../architecture.md) — layering rules 1–6; F26 device windows
