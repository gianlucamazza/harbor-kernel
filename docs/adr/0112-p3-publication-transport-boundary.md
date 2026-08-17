---
id: 0112
title: Publishing a NIC backend — one transport boundary, two backends
status: proposed
date: 2026-08-17
related: [0100, 0101, 0104, 0105, 0106, 0110]
---

# ADR-0112: The transport boundary that publishes a backend

## Status

**Proposed** (2026-08-17). Design for the roadmap's `next` row, _publish the Pi
4 backend to the network service_. Written before code, per
[ADR-0001](0001-multi-role-analysis.md); the code it authorises is the slice
that moves P3 from `done (QEMU)` to `done (HW)`.

## Context

[ADR-0105](0105-pi4-nic-backend-boundary.md) was accepted on 2026-08-17 with
hardware evidence for the **backend**: probe, link, a bounded TX confirmed by
UniMAC's counter and an `0x88b5` frame on the wire, a bounded RX,
reset/recovery, and an absent-device refusal. Acceptance did not cover
publication, and the product still prints:

```
authority: network vocabulary VACANT
```

on `raspi4b`, because `start_network_service()` in
`src/bootstrap/authority.rs` is `#[cfg(feature = "board-qemu-virt")]` and
returns `None` on every other board.

### What actually stands in the way

`src/bootstrap/network_runtime.rs` — the resident transport owner below the EL1
service — is virtio all the way down. It imports `virtio::PacketPool`,
`virtio_mmio::Configured`, `QueueMemory` and `QueueSetupFailure`, allocates the
virtqueue rings itself, and its ten public functions are what
`network_server.rs` calls.

`network_server.rs`, by contrast, is already transport-neutral in shape: it
speaks `kernel_core::net::Request` and moves tokens.

So the boundary ADR-0105 §4 requires — _"backend-specific packet formats are
translated at the EL1 boundary; the service does not silently assume virtio
headers or reuse virtio queue layout for another device family"_ — does not
exist yet. It has to be built before a second backend can be published, and
building it is most of this slice.

### The leak has a name

`PacketToken` is `{ slot: u8, generation: u32, len: u16 }`. `PacketError` is
`SlotOutOfRange | StaleGeneration | WrongDirection | WrongOwner | Oversize`.
Nothing in either is virtio. They live in `kernel_core::virtio` because that is
the file they were written in, and `network_server.rs` — the transport-neutral
service — imports `kernel_core::virtio::PacketToken` to do its job.

That import is ADR-0105 §4's failure in miniature, present today, on the
accepted QEMU path.

## Decision

### 1. The packet ABI moves to `kernel_core::net`

`PacketPool`, `PacketToken`, `PacketError` and the slot state machine move from
`kernel_core::virtio` to `kernel_core::net`, beside `Request` and `decode`.
`kernel_core::virtio` keeps what is genuinely virtio: the queue layout, the
feature negotiation, the descriptor arithmetic.

This is a rename with no behaviour, and it is the part of the slice that must
happen first: while the service's token type is named after one device family,
every later decision is made under a false constraint.

### 2. One board-selected transport, not a trait object

The backend is chosen the way every other board difference in this kernel is —
`crate::bsp::board::net`, a module with a fixed API, selected by `cfg` at build
time. The same shape as `crate::bsp::board::sdhci::init`.

Not a `dyn Transport`: the kernel has no allocator on this path, and a vtable
would put an indirect call in the packet path to express a choice that is
already made at compile time. Not generics either — one board builds one
kernel, and a type parameter threaded through `network_runtime` would buy
nothing a `cfg` does not.

The API is the smallest set `network_server.rs` actually needs, which today is:

| Function              | Meaning                                             |
| --------------------- | --------------------------------------------------- |
| `start()`             | claim the device and its DMA memory, or refuse      |
| `submit_tx(token)`    | hand a filled TX slot to the device                 |
| `return_rx(token)`    | hand an emptied RX slot back                        |
| `take_tx_complete()`  | a TX slot the device has finished with              |
| `take_rx_available()` | an RX slot the device has filled                    |
| `poll()`              | advance both directions                             |
| `reset()`             | drop every outstanding token and start a generation |

`network_runtime` keeps the pool, the frame ownership and the generation
counter — those are service state, identical for both backends — and delegates
only the device half.

### 3. The GENET backend consumes the model ADR-0110 declared

`src/drivers/genet.rs` is a bring-up driver: one descriptor at index 0, posted
once, polled to completion, no ring. The service needs a ring, and
[ADR-0110](0110-a-model-is-consumed-or-declared.md) has just finished declaring
`RingState`, `RingLayout` and `RingError` as _design-ahead for this slice_.

This is that slice. Those three stop being design-ahead and start being
consumed — which is the outcome ADR-0110's gate exists to make happen rather
than hope for.

`Genet::boot_after_program` stays where it is. It is the bring-up witness that
produced the ADR-0105 evidence, it runs before the service does, and deleting
it would delete the only thing that reports a NIC-less board honestly.

### 4. The first slice polls; interrupts are a later one

`network_runtime::poll()` already exists and is how the virtio path advances.
The GENET backend does the same. Taking a GENET interrupt means binding the
IRQ, acknowledging `INTRL2` and classifying the status word — a separate
boundary with its own evidence, and bundling it here would make a single slice
that cannot be bisected.

**Consequence for ADR-0110's annotations:** `InterruptWork` and `ResetState` are
_not_ consumed by this slice, and their doc comments say
`Design-ahead (P3 publication)` — which this ADR would make false. They are
re-pointed at the interrupt slice by name. Leaving them would be exactly the
drift `make model-consumed` was built to refuse, created by the ADR that cites
it.

### 5. QEMU `raspi4b` stays vacant, and that is the tested path

QEMU's `raspi4b` has no GENET: the machine deletes the node, and the product
prints `genet: unavailable (Missing)`. So publication changes nothing there —
`raspi4b` under QEMU keeps printing `network vocabulary VACANT`, through the
refusal path that already carries a hardware stamp (boot 8 of
`20260817-105728.log`, with the NIC-less boot description).

Stated here so that a green `make check` is never read as coverage of this
slice. **The only evidence for the GENET backend is on silicon**, and QEMU's
role is to prove the refusal, not the function.

## Evidence gate

P3 becomes `done (HW)` when one capture holds:

- `authority: 0 net-tx ok` … through `net-rx-return`, and **no** `network
vocabulary VACANT`, on a Pi 4B;
- an EL0 agent from the store sending through a granted cap, and the frame on a
  host pcap — the same `0x88b5`-class proof the backend gate used, but arriving
  from an agent rather than from bring-up;
- an EL0 agent receiving a frame the host sent, with the payload checked;
- a service reset that invalidates outstanding tokens, and a bounded refusal
  after it;
- on a boot description without `/scb/ethernet`
  (`scripts/host/absent-nic-dtb.sh`), the vocabulary vacant and the product
  otherwise healthy.

Plus, off silicon: host tests for the ring arithmetic through `RingState`, and
`make qemu-virtio-check` still green — the virtio path must not regress, since
this slice rewrites the module it depends on.

## Consequences

### Positive

- P3 closes, and with it H1.
- ADR-0105 §4's boundary stops being a requirement on paper and becomes a
  module.
- The virtio backend gets the same boundary, so the accepted QEMU path is
  tested through the abstraction rather than around it.

### Negative / costs

- **This rewrites the module the accepted P3 QEMU implementation depends on.**
  `make qemu-virtio-check` is the gate that says whether that survived, and it
  needs a host QEMU can actually run on ([ADR-0087](0087-oracle-waits-and-the-hosts-verdict.md)).
- The packet-ABI move touches `kernel_core::virtio`'s public surface and every
  importer, including `crates/kernel-core/tests/public_api.rs`.
- Two backends means two implementations of the same seven functions, and only
  one of them can be tested without a board on the desk.
- The mutation surface grows again, and #81 is not yet paid.

## Alternatives rejected

| Alternative                                 | Why not                                                                                                                                                        |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Add `cfg` branches inside `network_runtime` | Precisely what ADR-0105's decision rule forbids: _"the accepted virtio implementation is not generalized by adding board conditionals or compatibility shims"_ |
| `dyn Transport`                             | An indirect call in the packet path for a choice made at compile time, in a kernel with no allocator there                                                     |
| Publish first, factor later                 | The service would import `virtio::PacketToken` while driving GENET, which is the ADR-0105 §4 violation written down                                            |
| Include interrupts in this slice            | One slice that cannot be bisected, and the bring-up evidence took twenty-five bisections to earn                                                               |
| Leave `PacketToken` where it is             | A transport-neutral ABI named after one transport is how the next backend inherits the same confusion                                                          |

## The gate that catches its own reversal

- `make model-consumed` — `RingState` and friends must become reachable or keep
  a truthful declaration; this ADR cannot quietly not-happen while its
  annotations claim it did.
- `make qemu-virtio-check` — the accepted path still works through the new
  boundary.
- `make product-boot-check` on `raspi4b` — still `VACANT`, still honest.
- `make hw-check` plus a pcap — the gate above, on silicon.

## References

- [ADR-0105](0105-pi4-nic-backend-boundary.md) §4 — the boundary this builds
- [ADR-0104](0104-p3-edge-network-composition.md) — the accepted service ABI
- [ADR-0110](0110-a-model-is-consumed-or-declared.md) — the model this consumes
