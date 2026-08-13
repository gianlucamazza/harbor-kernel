---
id: 0102
title: The product binds a name — an agent finds the service
status: accepted
date: 2026-08-13
accepted: 2026-08-13
related: [0017, 0021, 0035, 0039, 0049, 0052, 0088, 0099, 0100, 0101]
---

# ADR-0102: The product binds a name — an agent finds the service

## Acceptance status

**Accepted** (2026-08-13), on delegated authority. The owner chose
_composable authority_ as the direction; [ADR-0099](0099-composition-vocabulary.md)
through [ADR-0101](0101-composed-driver-agent.md) were accepted the same way.
This slice is the next word that list already named: a name registry the
**product image** actually binds into, not only `demos.rs`.

## Context

The roadmap's next row names the console _server_ as what is still compiled
in. [M8](../design/m8-console-endpoint.md) decided that server is **EL1
infrastructure**, not a manifest entry (K1). Moving it into the store would
be a successor to that design: UART ownership, architecture rule 6, the
drain barrier, and `console-server: up` as a product-boot line. That is not
this slice.

[ADR-0099](0099-composition-vocabulary.md) already listed what the vocabulary
was for. One item — a device window a driver-agent could arrive holding —
landed as [ADR-0100](0100-device-windows.md)/[0101](0101-composed-driver-agent.md).
Two remain: storage reachable from EL0, and **a name the product image binds**.
P5's evidence today lives in `demos.rs`. An agent that can find a service
only because a demo bound it has not found a service the product offers.

[ADR-0052](0052-p5-resolve-grant.md) made `SYS_RESOLVE` a per-task grant, not
ambient. That grant is a boolean on the TCB, not a `CapId`, and this ADR
does not change that. A name-service capability is the alternative 0052
rejected; it stays rejected.

## Decision

### 1. M8 stands

`start_console_service` stays in `authority::assemble`. No store entry holds
the console recv end. The drain loop is not an agent.

### 2. The product publishes the console send end under a name

After the console capability is minted, `assemble` binds it:

```text
naming::bind(b"console", send)
```

and prints `authority: bound console`. A failed bind does not move any
`held` index — it is not a `held` position — and agents that resolve the
name are refused. The bind is a fact of the product boot, not of an oracle
demo.

The name is **`console`**, seven bytes. `SYS_RESOLVE` already carries up to
eight ([`unpack_name`](../../crates/kernel-core/src/reply.rs)); shortening it
to fit a 16-bit `movz` would be a workaround.

### 3. Resolve stays a grant, and the store can name the grant

Default remains false ([ADR-0052](0052-p5-resolve-grant.md)). The store
record's reserved word (ADR-0088: `home_cpu` in bits 7:0) grows **bit 8** =
`may_resolve`. Bits 31:9 stay zero and are still refused. Version stays **2**:
a boolean fits in a bit that was reserved, and a new version for one flag
would refuse every store this product already writes.

Granting resolve to every store agent would make resolve ambient in the
product. 0052 exists so that does not happen.

### 4. `lookup` arrives without the console slot

A store entry named `lookup` has every slot `SLOT_NONE` and `may_resolve`.
It resolves `console` into slot 0 and sends **`N`**. An encoder that dropped
the resolve changes the byte on the wire, the same way `entropy` proves it
read a register.

The oracle builtin table gains `noresolve`: the same image, the flag off.
It loads and runs; `SYS_RESOLVE` is refused; nothing it sends reaches the
wire. That is `mute` for the grant, as `nowindow` is `mute` for a window.

### 5. QEMU is the positive path

Unlike [ADR-0101](0101-composed-driver-agent.md), this slice does not depend
on a block the emulator lacks. `product-boot-check` requires `N` every day.
A hardware stamp confirms; it does not replace.

## Alternatives

| Option | Why not |
| --- | --- |
| Console server in the store | Supersedes M8; the conflict becomes the story |
| Storage on `held[1]` | Pays the P2 EL0 residual, but is a new protocol. After this slice, the product knows how to bind a name a later service can publish |
| A name-cap in `held[1]` | Rejected by ADR-0052; the boolean is enough |
| Grant resolve to every store agent | Ambient resolve in the product |
| Bind a two-letter name | The syscall carries eight bytes |

## The gate that would catch this ADR's reversal

`product-boot-check` requires `authority: bound console` and `N` on the wire
from `lookup`. A product that stops binding, or that grants `lookup` the
console slot instead of the name, fails one of those. `boot-check` requires
`noresolve` to have run with refusals and without a send, so a loader that
grants resolve by default fails the denial.

Going back to “names exist only in `demos.rs`” removes the bind line.

## Evidence

| Level | What |
| --- | --- |
| Host | Reserved-word bit 8 round-trips; bits 31:9 still refused; `encode_resolve_send_exit` against the `llvm-mc` oracle |
| QEMU | `authority: bound console`; `lookup` loaded and ran; `N` on the wire; `noresolve` ran with refusals and no send |
| HW | The same lines on a Pi 4B. Optional confirmation — the positive path is already green in `make check` |
