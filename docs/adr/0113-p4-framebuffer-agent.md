---
id: 0113
title: P4 — a framebuffer agent on a device window
status: proposed
date: 2026-08-17
related: [0026, 0049, 0094, 0100, 0101, 0111]
---

# ADR-0113: P4 is a framebuffer agent holding a device window

## Status

**Proposed** (2026-08-17), on delegated authority: the owner asked for the
analysis, the decision and the plan in one pass on 2026-08-17 and the decision
was recorded in issue #80 the same day. This ADR is the design that decision
requires before any code.

## Context

P4 — the product display/input path — has been `open` without a composition
target since 2026-08. [ADR-0094](0094-retire-debug-display.md) retired the lab
SPI panel on the grounds that a driver stack with no composition asking for it
is coverage theatre, and said the panel _"comes back when a composition names
it"_. Nothing named one.

Issue #80 offered two exits — scope, or permanent non-goal — and both were
refused:

- **Permanent non-goal** contradicts the project's own definition.
  [ADR-0026](0026-kernel-and-product-completeness.md) §1 lists _display/input_
  inside the meaning of the **P** track, and `vision.md` says display lands as
  an agent _"when a concrete composition needs"_ one. Excluding it needs an
  exclusion rationale arguing against both.
- **Indefinite deferral** is what [ADR-0049](0049-deferred-residuals.md)
  already records. It is honest and it settles nothing: it guarantees Harbor is
  never complete under its own definition.

So the decision was to name the composition, which neither exit did. **P4 is
the last item ADR-0026 counts.** Paying it makes Harbor complete by its own
measure; leaving it open means the project's completeness claim never closes.

## Decision

### 1. The composition is a `screen` agent holding a device window

Not a driver in the TCB. This is the third instance of an accepted pattern:

- [ADR-0100](0100-device-windows.md) declared the device-window vocabulary — a
  window is named by **index** into a declared list, never by a physical
  address on the wire, so a store cannot mint memory.
- [ADR-0101](0101-composed-driver-agent.md) put `entropy` on the `rng` window:
  the kernel probes, `provide_window` grants only if the block is there, and
  the agent reads `RNG_CTRL` before it speaks so a dropped grant changes the
  byte on the wire.

P4 is that shape again, with `framebuffer` declared beside `rng` and a `screen`
agent arriving in the store rather than compiled in.

It consumes code that already exists for exactly this. ADR-0094 removed the HAT
binding and **kept** `kernel_core::{display, textgrid, font8x8}` — Rgb565, the
text grid and the 8×8 font, pure and host-tested and already inside the
mutation scope — on the stated grounds that they are _"the part worth having
when a panel returns"_.

### 2. The VideoCore mailbox is EL1, and that is the real cost

Harbor has **no mailbox at all** today: `grep -ri mailbox` finds only the
ARM-local mailbox poke that brings up core 1 (`src/arch/aarch64/smp.rs`).
Getting a framebuffer means a property-channel client on the BCM2711 mailbox at
`PERIPHERAL_BASE + 0xb880`, and that is new TCB.

It stays in EL1 and is **not** offered as a window, because the property
channel is a firmware RPC that can _allocate memory and change the memory map_.
An agent holding it could ask the firmware for pages the kernel never granted —
minting memory through a side door, which is the exact thing ADR-0100 exists to
prevent. A window whose contents are a request channel is not a window.

So: the kernel asks the firmware for the framebuffer, and grants the **result**
— a fixed physical range — as an ordinary device window. The store names an
index; the address never appears in a manifest or an IPC message.

### 3. The window is provided only if the firmware answered

Same shape as `rng_present`: the boot's own probe decides. A board or emulator
whose mailbox does not answer, or answers with no framebuffer, leaves the
window declared and unprovided, and the `screen` agent refuses with
`authority: 1 framebuffer VACANT` rather than rendering into whatever address
it was handed.

### 4. The agent reads before it writes

ADR-0101's rule, applied to a surface instead of a register: the agent reads
the window's first pixel row back after writing it, and reports what it read.
An agent that renders into an ungranted mapping faults; one that renders into a
granted-but-wrong mapping reports a readback that does not match, instead of
silently painting nothing anyone will look at.

## The evidence problem, and the inversion it forces

This is the first track in the project where **the QEMU evidence is stronger
than the hardware evidence**, and the reason is worth stating rather than
discovering.

Every other track ends in bytes on a wire or a serial line, and hardware is
where the truth is. A framebuffer ends in _pixels_, and a Pi 4B cannot be asked
what is on its screen. QEMU can: `screendump` writes the emulated framebuffer
to a PPM the host can compare byte for byte.

So the gate is deliberately asymmetric:

| Level              | What it proves                                                                                                                                                                                                           |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Host**           | The Rgb565 conversion, the text grid arithmetic and the font, all already covered                                                                                                                                        |
| **QEMU `raspi4b`** | **The pixels.** The `screen` agent renders a known string; `screendump` is compared against a host-rendered PPM built from the same `font8x8`. This is the correctness evidence                                          |
| **Hardware**       | That the composition runs on silicon: the mailbox answers, the window is provided, the agent's readback matches, and it refuses when the window is not provided. Plus a photograph, **explicitly not machine-checkable** |

Naming that up front stops a later reader from treating the photograph as the
gate, and stops anyone from lowering the QEMU half because "hardware is what
counts here" — on this track, it is not.

**Open question the first slice answers:** whether QEMU's `raspi4b` implements
the property mailbox and allocates a framebuffer at all. If it does not, the
QEMU column above is unavailable and this ADR needs an amendment saying so
before the hardware column can be read as sufficient — not after.

## Consequences

### Positive

- ADR-0026's last open item gets a target, a design and a gate.
- 729 lines kept in reserve since ADR-0094 acquire a consumer, which is what
  [ADR-0110](0110-a-model-is-consumed-or-declared.md) asks of any model.
- The device-window vocabulary gets a second member, which is the first
  evidence that ADR-0100's index scheme generalises past one device.

### Negative / costs

- **A mailbox client is new TCB**, in a kernel whose whole argument is a small
  one. It is bounded — one channel, one tag list, no allocation — and it is
  still growth, and this ADR is where that is owned.
- The property interface is firmware behaviour, not silicon behaviour: a
  firmware bump can change what it returns. The blobs are pinned
  (`third_party/blobs`), so this is a versioned dependency rather than an
  unversioned one.
- One more window means one more thing a composition can name and get wrong.
- Nothing here is testable at all until the mailbox answers, so the first slice
  is a probe that reports and does not render — the same discipline the GENET
  bring-up used for twenty-five slices.

## Alternatives rejected

| Alternative                                | Why not                                                                                                          |
| ------------------------------------------ | ---------------------------------------------------------------------------------------------------------------- |
| Bring back the SPI panel                   | ADR-0094 retired it precisely to stop starting from a driver, and the binding would have to be rewritten anyway  |
| Grant the mailbox itself as a window       | A request channel that can allocate memory is not a window; it would let a store mint pages through the firmware |
| Render from EL1 and call it a display path | Then P4 is TCB growth with no claimant, which is what ADR-0094 removed                                           |
| Take the hardware photograph as the gate   | Not machine-checkable, and it would be the only track in the project whose evidence a person has to eyeball      |
| Defer again with a trigger                 | ADR-0049 already does; it is the status quo wearing a decision's clothes                                         |

## Evidence gate

P4 is `done (HW)` when:

1. `make product-boot-check` shows the framebuffer window declared, provided,
   and the `screen` agent's readback matching, on QEMU `raspi4b`;
2. a `screendump` from that boot matches a host-rendered PPM byte for byte;
3. a Pi 4B serial capture shows the same lines with a real firmware framebuffer,
   and `make hw-check` clean;
4. a boot with the window withheld shows `authority: 1 framebuffer VACANT` and
   the agent refusing, on both QEMU and silicon.

## References

- [ADR-0094](0094-retire-debug-display.md) — what was kept, and why
- [ADR-0100](0100-device-windows.md) / [ADR-0101](0101-composed-driver-agent.md)
  — the pattern this is the third instance of
- Issue #80 — the decision this designs
