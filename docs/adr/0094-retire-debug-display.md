---
id: 0094
title: Retire debug-display — the panel comes back with a composition, not before
status: accepted
date: 2026-08-11
accepted: 2026-08-11
related: [0009, 0010, 0020, 0026, 0049, 0091]
supersedes: [0009, 0010, 0020]
---

# ADR-0094: Retire `debug-display`

## Acceptance status

**Accepted** (2026-08-11), on delegated authority (structural improvement plan
approved by the owner on 2026-08-11, who confirmed the panel is not powered in
the lab; owner delegated acceptance for the slices that plan names).

## Problem

`docs/verification.md` has said it plainly for months: roughly 1.2k lines of
SPI/TFT/status code **compile and are never executed**. No oracle line, no
current hardware stamp — the 2026-08-08 stamp ran headless. `make check` spends
a target (`debug-display-builds`) proving it still compiles.

Underneath it, [ADR-0020](0020-spidevice-contract-without-a-caller.md) records a
`SpiDevice` trait with **no caller** — adopted for an XPT2046 touch controller
that never landed — kept on a standing watch (issue #14) that has not moved
since. And [ADR-0049](0049-deferred-residuals.md) defers P4, the product display
path, for want of a composition target that no roadmap cell asks for.

So: a driver stack with no product composition, a trait with no caller, and a
gate that proves compilation of code no execution ever reaches. Compiling is
the weakest coverage there is, and this is a project that measures coverage.

## Decision

Remove it. The panel comes back when a composition names it, written against
the SPI and DMA facts of that day rather than of 2026.

**Removed:** `src/status.rs`, `src/drivers/spi/`, `src/drivers/ili9486.rs`,
`src/drivers/pin.rs`, `src/bsp/rpi4/display.rs`, every
`cfg(feature = "debug-display")` block in `src/bsp/rpi4/{gpio,memmap,mod}.rs`,
`src/drivers/mod.rs`, `src/main.rs`, `src/bootstrap/mod.rs`,
`src/bootstrap/console_loop.rs` and `src/panic.rs`; the feature itself; the
`debug-display-builds` target and its place in `make check`; the
`FEATURES=debug-display` branches of `img` / `deploy`.

**Kept:** `kernel_core::{display, textgrid, font8x8, spi}` — around 730 lines
that are pure, host-tested and inside the mutation scope. Rgb565, the text
grid, the 8×8 font and the SPI clock-divider arithmetic are the part worth
having when a panel returns; what goes is the binding to one specific HAT, and
that binding would have to be rewritten anyway.

### Why not a QEMU gate instead

A `--features debug-display` boot under `raspi4b` is cheap and was the obvious
alternative. It is also **worse than the declared blind spot**: QEMU does not
model SPI0, so `BcmSpi` exhausts its spin limit and the boot prints
`display: init FAILED: Timeout`. That exercises the GPIO pinmux and one error
path — `ili9486.rs` (376 lines of command sequence) and `status.rs` (152 lines
of painter) stay at zero. It converts an honestly stated blind spot into a
green cell covering a quarter of the island. A gate that reports coverage it
does not have is worse than no gate.

### Why not a recurring hardware stamp

It would cover the island honestly, and it was the owner's call: the panel is
not powered during bring-up. A recurring obligation tied to specific physical
hardware, for a path no composition targets, is maintenance debt taken on to
protect code nothing runs.

Had the answer been the other way, the right shape was a **dated** stamp with
an explicit expiry, not a standing watch. ADR-0020 is the demonstration: a
permanent watch is a retirement that has not been scheduled.

## Consequences

- `discover: display compiled=on|off` (`src/bootstrap/discover.rs`) now always
  reports `off`. The line **stays**, and so does the assertion in
  `qemu-product-boot-check.sh`: discovery reports what the image claims, and
  "no display compiled in" is a true claim about this image. Removing the line
  would remove the place where a future panel announces itself.
- `make check` loses one target. The count in `README.md` and the gate list
  `doc-claims` compares move with it.
- Three ADRs become `superseded`: 0009 (the SPI TFT status surface), 0010 (SPI
  transactions and DBI panel streaming) and 0020 (the callerless contract).
  Their reasoning stands as a record of what was learned; what they describe is
  no longer in the tree.
- Issue #14 closes, citing this ADR.

## Gates

| Check                     | Evidence                                                                                                         |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Nothing left behind       | no `debug-display` in `src/`, `Cargo.toml` or the `Makefile`; `make check` green with one target fewer           |
| The pure half survives    | `kernel_core::{display, textgrid, font8x8, spi}` tests unchanged and still in the mutation scope                 |
| The claim is honest again | the `verification.md` blind-spot row is deleted rather than reworded — the surface it described no longer exists |
