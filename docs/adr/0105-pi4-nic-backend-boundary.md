---
id: 0105
title: Pi 4 NIC backend boundary and evidence gate
status: accepted
date: 2026-08-13
accepted: 2026-08-17
related: [0104, 0100, 0101, 0102, 0106]
---

# ADR-0105: Pi 4 NIC backend boundary and evidence gate

## Status

**Accepted 2026-08-17** by the project owner, on the evidence below. This ADR
records the work required before Harbor can claim a network backend on
Raspberry Pi 4B; the capture it asks for now exists, and the boundary it
draws is in force. Immutable under the ADR lifecycle: change only via a
successor ADR.

**What acceptance does and does not settle.** `raspi4b` has a NIC backend and
its hardware evidence. It does **not** publish one: the backend still sits
below the EL1 network service, the product still prints
`authority: network vocabulary VACANT`, and binding it to the network
vocabulary is the later BSP composition step this ADR names. P3 is therefore
not `done (HW)` on the strength of this acceptance — what is paid is the
backend and its evidence, not the composition.

Capture `20260817-105728.log`, product image, `make hw-check` clean:

| Gate item | Where | What the board said |
| --- | --- | --- |
| probe | boot 7, `src=0a937a23` | `genet: rev=6.0 patch=0x0 (mmio, not a nic)` |
| link state | boot 7 | `genet: phy=0x600d84a2`, `genet: link=down (bmsr, not a nic)` at probe; up at submit |
| one bounded TX | boot 7 | `genet: tx cons len=124 (dma, not a nic)` and `genet: umac tsv packed=0 linux=1 pok=1 (mib, not a nic)`, with `02:00:00:00:00:01 > ff:ff:ff:ff:ff:ff ethertype 0x88b5 length 60` in `20260817-105728-apple-nic.pcap` |
| one bounded RX | boot 7 | `genet: rx complete len=168 (one frame, not a nic)` |
| reset / recovery | boot 7 | `genet: reset recovered (idle, not a nic)` |
| absent-device refusal | boot 8, `src=a4b40bb3` | `genet: unavailable (Missing)` and `genet: probe unavailable (no binding)`, with a boot description built from the tracked fixture minus `/scb/ethernet` |

Necessarily two boots: a boot description cannot say the device is both
present and absent. The two images differ by commits that touch no kernel code
(`git diff --stat 0a937a23 a4b40bb3 -- src crates` is empty).

The road to that capture — thirty-one silicon stamps, twenty-five of them
single registers that changed nothing, and the two defects that did (UniMAC
programmed into a running DMA engine, and UniMAC left in software reset for
the whole boot) — is recorded boot by boot in
[verification](../verification.md#hardware-evidence-pi-4-genet-v5-bring-up-2026-08-14--2026-08-17).
The method change that found them is [ADR-0107](0107-genet-sequence-first-bring-up.md);
the link decision is [ADR-0108](0108-boot-path-link-acquisition.md).

**Until this ADR is accepted**, `raspi4b` has no NIC backend, P3 stays
`done (QEMU)`, and the product keeps printing
`authority: network vocabulary VACANT`.


## Context

ADR-0104 selects QEMU `virt` plus modern virtio-net as the first reproducible
P3 target. Raspberry Pi 4B does not expose that virtio-mmio device in its
current board path. Treating the QEMU transport as a Pi 4 NIC would create a
wrong hardware claim and would bypass the board's actual reset, interrupt,
cache, DMA, and ownership contracts.

## Required boundary

The board-specific design is recorded in [ADR-0106](0106-pi4-genet-v5-backend-design.md).

Any Pi 4 backend must remain below the existing EL1 network service:

1. The board backend discovers its device from the boot-time hardware
   description and refuses an absent, ambiguous, or incompatible device.
2. EL1 owns MMIO, rings/descriptors, DMA buffers, interrupt acknowledgement,
   cache maintenance, reset, and link state.
3. The EL0 packet service receives the same directional token protocol and
   packet-pool grant as ADR-0104. No board-specific register, physical
   address, or DMA pointer enters an agent manifest or IPC message.
4. Backend-specific packet formats are translated at the EL1 boundary. The
   service does not silently assume virtio headers or reuse virtio queue
   layout for another device family.
5. A failed probe or recovery leaves the network vocabulary vacant or reports
   a bounded service refusal; it does not fall back to a different device
   model without an explicit probe result.

## Evidence gate

Implementation may begin only after a board-specific design captures:

- the exact compatible/device identity and register map from the Pi 4 boot
  description or authoritative silicon documentation;
- reset, clock, interrupt, descriptor ownership, DMA addressability, and
  cache-coherency requirements;
- a minimal host model for ring arithmetic, malformed descriptors, ownership,
  and reset generations;
- a QEMU or emulator test where available, clearly labelled as non-hardware
  evidence;
- a serial capture on a real Pi 4 proving probe, link state, one bounded TX,
  one bounded RX, reset/recovery, and absent-device refusal.

The capture must identify the exact image, board revision, boot description,
kernel commit, and test input. A successful build, a QEMU virtio log, or a
USB/network host link alone is not hardware evidence for this ADR.

## Decision rule

Until the evidence gate is met, `raspi4b` keeps no NIC backend and P3 status
remains QEMU-only. The accepted virtio implementation is not generalized by
adding board conditionals or compatibility shims. A future backend gets its
own ADR or an explicit amendment with its own host, emulation, and hardware
evidence.

## Consequences

The current product claim remains honest and the network service ABI remains
portable. The cost is that Pi 4 network support is an explicit follow-up,
not an implied benefit of the QEMU implementation.
