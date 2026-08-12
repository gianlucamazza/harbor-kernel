---
id: 0100
title: Device windows — the composition names, the board decides
status: accepted
date: 2026-08-12
accepted: 2026-08-12
related: [0013, 0017, 0021, 0027, 0029, 0034, 0043, 0049, 0088, 0099]
---

# ADR-0100: Device windows — the composition names, the board decides

## Acceptance status

**Accepted** (2026-08-12), on delegated authority: the owner chose _composable
authority_ as the direction and approved a plan whose next step is this ADR.
[ADR-0099](0099-composition-vocabulary.md) was accepted the same way and is the
slice this one continues. The design choices inside it are the plan's; the
direction is the owner's.

## Context

A driver-agent needs one thing the manifest cannot give it: a page of MMIO.
The mechanism to hand one over already exists and has since
[ADR-0013](0013-narrow-device-windows.md):

```rust
// crates/kernel-core/src/manifest.rs
pub struct DeviceGrant {
    pub va: u64,   // inside the agent's own window
    pub pa: u64,   // the device page
}
pub device: Option<DeviceGrant>,
```

and the loader acts on it — `map_device_page(grant.va, grant.pa,
Perms::USER_RW)` before the agent is entered.

**Nothing ever sets it.** The built-in manifest writes `device: None` three
times, and `agentstore::to_entry` writes it once more, because the store's wire
format has no such field. The field has been dead since it was written.

So the driver-agents that exist reach their hardware the only way left: compiled
in. `demos.rs` calls `map_device_page(USER_RNG_VA, RNG200_BASE, …)` for the RNG
agent ([ADR-0034](0034-k9-rng-driver-agent.md)) and
`map_device_page(USER_PL011_VA, UART0_BASE, …)` for the PL011 one. Those are
agents in every sense the kernel cares about — isolated, EL0, killable — and in
no sense the product cares about: you cannot compose one, only rebuild with one.

This is the whole of why [ADR-0049](0049-deferred-residuals.md) defers P3 and
P4 for want of a composition target. A network agent is a driver-agent. There is
no way to express one in a store, so there is nothing to aim at.

## Problem

The obvious next move is to put the field on the wire — add `pa` to the store
record beside `va`, and let a composition say which page it wants.

That would make the store able to **mint**.

`map_device_page` maps what it is told, `Perms::USER_RW`, into an EL0 window. A
store record carrying a physical address is a store record that can name the
kernel's own text, its page tables, another agent's stack, or the frame pool —
and the loader would map it, print `loaded`, and enter EL0 with the kernel's
memory writable from user space. The store is trusted boot input
([ADR-0027](0027-h1-external-agent-store.md)), but "trusted" there means _we
accept it decides which agents run_, not _we accept it decides what physical
memory means_.

This is the same failure [ADR-0021](0021-agents-as-data-and-the-manifest.md)
designed out for capabilities, and it is worth being precise about why the fix
there worked. A manifest cannot mint a capability because it does not carry
capabilities — it carries **indices into a list the loader already holds**, and
the refusal for anything outside that list is `index >= held.len()`. Arithmetic,
not a policy check. Nobody has to remember to write it, and no future edit can
forget it.

A `pa` on the wire has no such structure. Guarding it would mean a check —
_is this address inside a range we consider a device?_ — and that check has
every property the manifest was built to avoid: it lives somewhere, someone
maintains it, and the day it is wrong nothing else refuses.

There is a second objection, and it is the architecture's rather than the
threat model's. Rule 1 of `architecture.md` is **"drivers never know the board
(bases / IRQ ids from BSP)"**. A physical address inside a store blob is a
board map that has escaped the BSP into external data — the exact knowledge the
layering exists to keep in one place, now packed by a host script.

## Decision

**A device window is named by index, exactly as a capability is.** The
composition chooses _where_ in its own window the page lands; the board decides
_what_ page that is and _with what rights_. Neither can do the other's half.

### 1. A second vocabulary, the same mechanism

[ADR-0099](0099-composition-vocabulary.md) gave the loader a vocabulary of
capabilities: declared positions, filled or vacant, indexed by the composition.
Device windows get the same treatment and the same code — `held::Set` becomes
generic over what a position holds:

| Vocabulary          | Position holds                     | Named by                    |
| ------------------- | ---------------------------------- | --------------------------- |
| `held::Set<CapId>`  | a capability                       | `slots[i]` of a store entry |
| `held::Set<Window>` | `Window { pa: u64, perms: Perms }` | the entry's `device` field  |

`declare` / `provide` / `as_slice` / `name_of` and their refusals are unchanged
— including the property that matters most, that a window which fails to come up
leaves a **hole** rather than shifting every later index down. The tests written
for ADR-0099 cover both vocabularies once `Set` is generic; the one thing added
is that `Window` is `Copy` data rather than a handle, which changes nothing
about the discipline.

`MAX_WINDOWS = 4` to start, against `MAX_HELD = 8`. Both are cheap to raise and
neither is a security bound — the bound is that an index outside the vocabulary
is refused by arithmetic.

### 2. The wire carries an index and a VA, never a PA

The store record's device field is:

| Field    | Width | Meaning                                                         |
| -------- | ----- | --------------------------------------------------------------- |
| `window` | u8    | Index into the window vocabulary; `WINDOW_NONE = 0xFF` for none |
| `va`     | u64   | Where in the agent's own window the page appears                |

The `va` is the composition's to choose because it is the composition's address
space — it is bounded by the agent's window and validated as any other geometry
is. The `pa` is not on the wire at any width, so there is no encoding of it a
malformed store could reach.

`DeviceGrant` changes shape to match: `{ va: u64, window: u8 }`. The loader
resolves `window` against the vocabulary and maps the `pa` and `perms` it finds
there, which means `Perms::USER_RW` stops being a constant at the mapping site
and becomes a property of the declared window — a read-only device is now
expressible, and was not.

### 3. `bootstrap::authority` declares the windows too

The windows live where the capabilities live, in `src/bootstrap/authority.rs`,
and take their addresses from the BSP as every board fact does:

```rust
pub const WINDOW_RNG: u8 = 0;
// order is an ABI shared with scripts/agent/pack-store.py
```

`assemble` returns both vocabularies. A window whose device is not present on
this board — the RNG200 probe already reports `unavailable (NotPresent)` on
QEMU — declares its position and provides nothing, so the boot names the
vacancy and a composition asking for it is refused by name instead of mapping a
page that is not there.

**This product declares no window**, and that is the shipped state rather than
an unfinished one: granting a device is a composition decision, and the first
composition that needs one arrives with the first composed driver-agent, its own
ADR and its own slice. So the boot says `authority: windows 0 declared` — the
absence stated rather than inferred — and every entry naming a window is refused
by `index >= 0`.

That refusal is not left to a document. The oracle's built-in manifest gains
`nowindow`: `beacon`'s own bytes with a device grant naming window 0, so every
oracle boot prints

```text
loader: nowindow refused — names window 0 of 0
```

which is the same trick `mute` plays for capabilities — the denial seen working
on the good path, rather than argued for.

### 4. Refusals stay two, and stay distinct

`BindError` already distinguishes _you named something outside the vocabulary_
from _you named a position nobody filled_. Device windows reuse both, with the
device's own names:

| Refusal                       | Meaning                                      |
| ----------------------------- | -------------------------------------------- |
| `NoSuchWindow { index, len }` | Past the end of the window vocabulary        |
| `WindowVacant { index }`      | Declared, and nothing was provided this boot |

Both are `loaderplan::Refusal::Unheld`, for the reason ADR-0099 §2 gives: that
enum partitions by _which check refused_, and both of these are `bind`.

## Alternatives

| Option                                             | Why not                                                                                                                                                                                                                                                 |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pa` on the wire, validated against a device range | The store can mint. A range check is a policy check — it lives somewhere, and the day it is wrong nothing else refuses. ADR-0021 removed exactly this shape for capabilities                                                                            |
| `pa` on the wire, and trust the store              | "Trusted boot input" licenses the store to choose which agents run, not to redefine what physical memory means. The blast radius is the whole kernel, from one host script                                                                              |
| Name windows by string (`device: "rng"`)           | Squattable, 16 bytes a name in a 16 KiB store, and it moves the decision from compose time to boot time. Same argument ADR-0099 made against names for capabilities                                                                                     |
| A device **capability** in the slot table          | Bigger and not obviously better: a window is mapped into the address space at spawn, not held and invoked, so it is not the same kind of thing a `CapId` is. If a future service needs to _pass_ a device on, that is when a capability earns its place |
| Keep compiling driver-agents in                    | It is the status quo, and it is what P3/P4 are waiting on. An OS where software arrives as agents cannot have its drivers arrive as rebuilds                                                                                                            |

## Consequences

- `DeviceGrant`'s shape changes. It has no users today, so the change costs one
  struct and the four `device: None` sites.
- `held::Set` becomes generic. The ADR-0099 tests move with it; none are
  rewritten, which is the point of reusing the mechanism rather than writing a
  second one.
- The store format gains a field, so `VERSION` goes to 2 and the parser refuses
  1 — there is no compatibility to keep, since no store in existence sets it.
  The reserved u32's high bits (ADR-0088 left 31:8 zero) are **not** where this
  goes: a window index and a VA do not fit in three bytes, and reusing a
  reserved field for something structural is how formats become unreadable.
- `Perms` reaches `kernel-core`'s manifest types, which it already does through
  `paging::Perms` — no new layering edge.
- This ADR grants no device to anything. It is the second word of the
  vocabulary; the first composed driver-agent is the sentence.

## The gate that would catch this ADR's reversal

`make vocabulary-sync` grows a second table and compares it the same way, so a
window index that means one thing to the kernel and another to the packer fails
the build rather than the boot. It was **seen red** on exactly that drift
(kernel `rng 0`, packer `rng 1`) and green when the two agree — worth saying
because the first version of the comparison could not fail: it anchored the
table name with a trailing space, found nothing in `WINDOWS: dict[str, int] =
{}`, and reported clean by comparing empty against empty.

The reversal that matters — putting a `pa` back on the wire — is caught by a
host test asserting that `agentstore::parse` has no path from store bytes to a
physical address: the parsed record's device field is an index, and the only
`u64` beside it is bounded by the agent's window. A `pa` field reintroduced to
the format makes that test fail to compile, which is the loudest red available.

`product-boot-check` asserts `window:` lines the same way it now asserts
`authority:` ones, and refuses `VACANT` among the negatives.

## Evidence

| Level | What                                                                                                                                                                                                                                                                                       |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Host  | `Set<T>` generic with the ADR-0099 properties intact for both instantiations; an index past the vocabulary refused as `NoSuchWindow` and a hole as `WindowVacant`, asserted **distinct**; a store record round-tripping `window` + `va`; the format-level test that no `pa` can be encoded |
| QEMU  | `authority: windows 0 declared` in `make product-boot-check`; `loader: nowindow refused — names window 0 of 0` in `make boot-check`, with the negatives that a refused entry was neither loaded nor ran — an agent composed to drive a page it cannot have does not run without it |
| HW    | Deferred to the first composed driver-agent: a vocabulary with no window declared has nothing to stamp on silicon that QEMU does not already show                                                                                                                                          |
