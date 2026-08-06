---
id: 0010
title: SPI transactions and DBI panel streaming (no full-frame alloc)
status: accepted
date: 2026-08-05
accepted: 2026-08-05
---

# ADR-0010: SPI transactions and DBI panel streaming

## Acceptance status

**Accepted.** Extends [ADR-0009](0009-optional-spi-tft-debug-console.md) with the
bus-session shape required by ILI9486 RAMWR (and any long SPI slave burst).

## Context

`SpiDevice::write` asserts CS for one contiguous buffer and releases it. That
matches embedded-hal 1.0 for short register access. It is **wrong** as the only
primitive for ILI9486 pixel streams:

- RAMWR requires **one CS assertion** for the entire GRAM payload (or at least
  for a continuous data phase after the command).
- Calling `write` per FIFO-sized chunk deasserts CS between pieces → partial
  GRAM update → white band (observed on silicon).
- Allocating a full 480×320×2 frame (~300 KiB) just to call `write` once is a
  **workaround**: it restores CS discipline by accident of buffer size, burns
  heap, and fights ADR-0009’s “no full-frame requirement for status text”.

Modern practice (e-hal `SpiDevice` + bus session / `transaction`, MIPI DBI
command/data split) is: **hold CS across multiple bus operations**, toggle DC
between command and data, stream pixels from a small repeated pattern or dirty
window — never require a full framebuffer for a solid fill.

## Decision

### 1. Transaction API on the software-CS device

`ExclusiveDevice` exposes:

```text
with_bus(|bus: &mut impl SpiBus| { ... })
```

Semantics:

- assert CS (active low);
- run `body` against the **raw bus** (no further CS toggles);
- always deassert CS (and optional idle delay), even on error.

Short register ops keep using `SpiDevice::write` / `transfer` (one CS each).
Long streams (RAMWR, bulk flash) use `with_bus`.

ILI9486 **must not** bit-bang CS. It either uses `SpiDevice` for short ops or
`with_bus` when it holds an `ExclusiveDevice` (or a future trait that exposes
the same session).

### 2. DBI-shaped panel driver

Panel I/O is command/data with an explicit DC pin:

| Step | DC | Payload |
| ---- | -- | ------- |
| Command | 0 | opcode byte(s) |
| Data | 1 | parameters or RAMWR pixels |

Init remains a declarative [`InitOp`] table (datasheet-first). Orientation and
geometry live in a small `PanelGeometry` / MADCTL value, not magic in fill.

### 3. Streaming fill — no full-frame buffer

Solid colour fill:

1. `set_window` full panel;
2. `with_bus` {
     - DC=0, write RAMWR;
     - DC=1, loop: write a small stack buffer filled with the RGB565 colour
       (FIFO-aligned chunks, e.g. 32–64 pixels) until pixel count is met;
   }
3. CS released once at the end of `with_bus`.

Partial/window updates for the future status surface use the same path with a
smaller window and only dirty pixels — still no mandatory full GRAM mirror.

### 4. Self-test

SPI controller self-test stays **CS high** (slave not selected) or panel held
in reset. It must not emit bus noise that looks like a command stream on a live
panel.

### Explicit non-goals

- DMA SPI (separate decision when measured);
- embedded-graphics / full compositor;
- **SPI controller 16-bit mode** (BCM word size / mode bits) unless a named SKU
  proves 8-bit buswidth insufficient — do not confuse with **regwidth-16
  framing** below;
- Feature-gating around missing silicon (use `arch::probe` where absence is
  expected, as for RNG200).

### SKU wire framing (Waveshare / PiScreen)

fbtft `piscreen` uses `regwidth=16` on `buswidth=8`: every **command** and
**parameter** byte is sent as a big-endian `u16` with high byte zero
(`0x00, opcode`). Pixel RGB565 words are raw 16-bit colour (not zero-padded).
That is still SPI mode 0, 8-bit words on the BCM block. Bare 8-bit commands on
that HAT class produce noise / faint lines on glass (observed 2026-08-05);
reg16 framing is part of the Waveshare BSP profile, not a protocol workaround.

## Consequences

### Positive

- CS discipline is structural, not “hope the buffer is one write”.
- Heap stays free of 300 KiB demo frames.
- Status-surface path can share the same streaming primitive.
- Aligns with e-hal session thinking without taking the crate as a dep.

### Costs

- Ili9486 fill needs a path to `with_bus` (concrete `ExclusiveDevice` or a thin
  session trait). Generic `SpiDevice` alone is not enough for multi-write
  under one CS — that is intentional: the short-write trait stays simple.
- Register writes expand each logical byte to two wire bytes (SKU profile).

### Gate that would catch a reversal

| Reversal | Signal |
| -------- | ------ |
| Chunked `SpiDevice::write` for RAMWR again | White band / incomplete fill on silicon |
| Full-frame alloc for solid fill | ~300 KiB heap spike at boot |
| CS toggled inside `ili9486` | Layering / ADR-0009 hard rule |
| Silent panel failure | No `display: …` line; boot still continues |
| Drop reg16 framing on Waveshare HAT | Gray noise / faint lines instead of solid colours |

## Related

- [0009](0009-optional-spi-tft-debug-console.md) — overall display side-track
- [`../hardware.md`](../hardware.md) — pinout and SKU notes
- Linux `fb_ili9486` / fbtft `piscreen` — init + regwidth cross-check only
