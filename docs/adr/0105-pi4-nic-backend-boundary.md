---
id: 0105
title: Pi 4 NIC backend boundary and evidence gate
status: proposed
date: 2026-08-13
related: [0104, 0100, 0101, 0102, 0106]
---

# ADR-0105: Pi 4 NIC backend boundary and evidence gate

## Status

**Proposed.** This ADR records the work required before Harbor can claim a
network backend on Raspberry Pi 4B. It deliberately does not select a device
or claim that a driver exists.

A 2026-08-14 Pi 4B oracle boot stamp confirms the board, UART path, image
provenance, SMP, and durable-media baseline. The product prints a `genet:`
FDT report and, when that binding matches the compiled window, runs
`Genet::probe`. Silicon stamp `20260814-174803.log` boot 3 (`src=f7404550`) has the
FDT line, `rev=6.0`, `phy=0x600d84a2`, `link=down`,
`genet: queue0 programmed (rings, not a nic)`, and `VACANT`.
Programmed rings with DMA disabled are not this ADR's NIC gate. No
TX/RX, service reset/recovery, or absent-device result was produced.

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
