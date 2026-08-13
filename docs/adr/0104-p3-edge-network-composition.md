---
id: 0104
title: P3 edge-gateway composition over virtio-net
status: accepted
date: 2026-08-13
accepted: 2026-08-13
related: [0017, 0013, 0021, 0035, 0049, 0056, 0099, 0103]
---

# ADR-0104: P3 edge-gateway composition over virtio-net

## Acceptance status

**Accepted** (2026-08-13), on delegated authority. This ADR opens P3 with a
named composition target; it does not claim that the Raspberry Pi 4B NIC is
implemented.

## Context

P3 has remained deferred because “network agent” was not a product target.
An edge gateway is now the target: an untrusted EL0 forwarding/policy agent
uses a narrow network service, while the device driver and packet ownership
remain in EL1. The first deterministic target is AArch64 QEMU `virt` with a
modern virtio-net device. The existing Pi4 `raspi4b` product path has no
virtio-net device and must not be made to claim one.

The boundary must preserve Harbor's existing rule: an agent names authority by
slot and receives no raw MMIO address, DMA address, or kernel-owned pointer.

## Decision

### Target and driver ownership

The first P3 composition is `edge-gateway` on AArch64 QEMU `virt`:

1. EL1 owns the virtio-mmio transport, feature negotiation, split virtqueues,
   interrupt handling, descriptor recycling, and DMA memory.
2. The driver uses the modern virtio transport and refuses legacy-only devices
   or unsupported feature combinations; no implicit feature fallback is used.
3. The EL0 agent owns policy and forwarding decisions. It never maps the
   virtio registers or device descriptor rings.

### Capability boundary

The composition vocabulary exposes directional endpoints:

- `net-tx`: agent SEND, service RECV;
- `net-rx`: agent RECV, service SEND;
- `net-rx-return`: agent SEND, service RECV.

The service also receives a packet-pool grant. The pool is a bounded set of
fixed-size 2 KiB slots, split into TX and RX regions. EL0 may write TX slots
and read RX slots; it cannot access the service's DMA ring or the device's
MMIO window. A descriptor message carries only an operation, pool slot index,
length, and generation. The service rejects an out-of-range slot, stale
generation, oversize frame, double return, or wrong-direction operation before
touching virtio state.

The service copies TX data from the agent pool into EL1-owned DMA buffers and
copies received frames into RX slots before notifying the agent. This is the
first-slice safety boundary; zero-copy DMA into agent memory is explicitly not
part of P3.

### Lifecycle and failure policy

The service is started by authority assembly and remains EL1 infrastructure,
like the console and blob services. If the device is absent or feature
negotiation fails, the vocabulary positions remain vacant and the loader
refuses agents that name them. A device reset drains and invalidates all
outstanding generations before the service advertises readiness. No packet is
silently reused after reset.

## Consequences

P3 now has a reproducible composition target and an explicit least-authority
shape. The packet pool adds a memory-grant primitive and accounting pressure;
the service must copy frames, so this is a safety-first first slice rather than
a throughput claim. The Pi4 product remains honest until a separate NIC
backend and hardware evidence exist.

## Alternatives

| Option | Why not |
| --- | --- |
| Map virtio MMIO and rings into EL0 | Exposes device authority and DMA ownership to policy code |
| DMA directly into agent memory | Requires a stronger IOMMU/ownership proof than the current kernel provides |
| Add network syscalls | Makes one device family a kernel ABI instead of a service boundary |
| Start with the Pi4 NIC | No current product NIC driver or deterministic hardware contract exists |
| Use one bidirectional endpoint | Couples TX/RX rights and makes backpressure and lifecycle ownership ambiguous |

## Evidence required for the implementation successor

| Level | Required evidence |
| --- | --- |
| Host | virtio feature negotiation, queue arithmetic, slot/generation ownership, malformed descriptor rejection, reset invalidation |
| QEMU | `edge-gateway` composition on AArch64 `virt`; TX/RX loopback or deterministic peer; absent-device refusal; no raw MMIO/DMA grant to EL0 |
| Hardware | Separate Pi4 NIC backend ADR and serial/network capture; no QEMU evidence is promoted to Pi4 status |
