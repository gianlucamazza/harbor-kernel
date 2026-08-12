---
id: 0099
title: The composition's vocabulary — a declared held list
status: accepted
date: 2026-08-11
accepted: 2026-08-11
related: [0001, 0017, 0021, 0026, 0027, 0029, 0049, 0052, 0097, 0098]
---

# ADR-0099: The composition's vocabulary — a declared `held` list

## Acceptance status

**Accepted** (2026-08-11), on delegated authority: the owner asked for ideas on
finishing the product, chose _composable authority_ as the direction, and
delegated the composition target to the plan.

## Context

[ADR-0021](0021-agents-as-data-and-the-manifest.md) makes an agent's authority
an **index**: a manifest entry names `slots[i] = k`, and `manifest::bind` turns
`k` into a `CapId` by indexing the loader's `held` list. Nothing outside that
list is reachable, so a manifest cannot mint — the refusal is arithmetic
(`index >= held.len()`), not a policy check.

The mechanism is right and the vocabulary is one word long:

```rust
// src/bootstrap/mod.rs
let one;
let held: &[kernel_core::cap::CapId] = match console_cap {
    Some(cap) => { one = [cap]; &one }
    None => &[],
};
loader::load_all(held);
```

The comment above it already says what this ADR is about: _"`held` here is the
whole of what any manifest agent can be given"_. Today that is the console send
end and nothing else. Every product service the roadmap still owes — storage
reachable from EL0 (P2 residual), a device window a driver-agent could be
**composed** with rather than compiled with, a name registry the product image
actually binds into (P5's evidence lives in `demos.rs`) — needs a second entry
in that list, and a third.

## Problem

Lengthening that list with the same pattern introduces a bug that no test in the
tree would catch.

With one entry, a failed mint yields an empty list: every agent is refused with
`NoSuchCapability`, loudly and correctly. With four entries assembled by
pushing what succeeded:

```rust
let mut held = [console?, blob?, blob_rx?, window?];   // whatever was minted
```

a failed console mint **shifts every later index down by one**. A store entry
that says `slots[1] = 0` — meaning _console_ to whoever composed it — is bound
to the storage endpoint instead, silently, and the agent runs. That is
capability confusion produced by a missing element, and it is invisible: the
boot log says the agent loaded, the loader says `refusals=0`, and the wrong
authority was granted by arithmetic that was correct on its own terms.

Three properties are missing, and each is the cause of a different failure:

1. **Positions are not stable.** An index means "the *n*th capability that
   happened to exist this boot" rather than a name the composition can rely on.
2. **A vacancy is indistinguishable from a mistake.** `NoSuchCapability` says
   _you asked for something that does not exist_. A minted-nothing says _you
   asked for something that should exist and does not_ — a boot-time failure of
   the kernel, not a mis-composition, and the console line should not conflate
   the two.
3. **The vocabulary has no owner.** It is three lines in the middle of
   `bootstrap::run`, so adding a service means editing the boot sequence, and
   the packer's idea of what index means what is written down nowhere.

## Decision

### 1. A vocabulary is declared, then provided (pure)

New module `kernel_core::held`:

| Operation                                         | Meaning                                                            |
| ------------------------------------------------- | ------------------------------------------------------------------ |
| `declare(name) -> Result<u8, DeclareError>`       | Reserve a **position** and return its index. Nothing is minted yet |
| `provide(index, cap) -> Result<(), ProvideError>` | Fill a declared position                                           |
| `as_slice() -> &[Option<CapId>]`                  | What the loader binds against                                      |

`DeclareError::{Full, Duplicate}`, `ProvideError::{OutOfRange, AlreadyProvided}`.

**Declaration and provision are separate calls, and that separation is the whole
decision.** An index is assigned before anyone knows whether the mint will
succeed, so a failure leaves a **hole**, not a shift. `MAX_HELD = 8`, with
`const _: () = assert!(MAX_HELD < agentstore::SLOT_NONE as usize)` so the
sentinel `0xFF` can never collide with a legal index.

Names are carried beside the caps. They cost 16 bytes a position and they are
what makes a vacancy printable — `authority: 1 blob VACANT` names the service
that did not come up, where an index alone would need a reader to go and count.

### 2. `bind` takes a slice of `Option`, and a vacancy is its own refusal

`manifest::bind(entry, held: &[Option<CapId>])`, with a new variant:

| Refusal                                             | Meaning                                                                                                              |
| --------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `BindError::NoSuchCapability { slot, index, held }` | The index is past the end of the vocabulary. **Unchanged** — the composition asked for something that does not exist |
| `BindError::HeldVacant { slot, index, name }`       | The index is in the vocabulary and nothing was minted into it this boot                                              |

Both wrap as `loaderplan::Refusal::Unheld`. **No third `Refusal` variant**: that
enum partitions by _which check refused_ — `validate` or `bind` — and a vacancy
is a `bind` refusal. Adding a variant per kind of authority would make the
partition mean two things at once.

### 3. The vocabulary has one owner, and `run` shrinks

New module `src/bootstrap/authority.rs`, the only place the product's vocabulary
is written:

```rust
pub const HELD_CONSOLE: u8 = 0;
// order is an ABI shared with scripts/agent/pack-store.py
pub fn assemble() -> held::Set
```

`assemble` declares every position, mints into the ones it can, and prints one
line per position — `authority: 0 console ok`, or `authority: 1 blob VACANT
{e:?}`. `bootstrap::run` becomes `loader::load_all(authority.as_slice())`, and
adding a service is an edit to `authority.rs` rather than to the boot sequence.

"Only place" is meant literally, and the built-in manifest is the test of it:
`loader.rs` grants the beacon its console by index exactly as a store entry
does, so it imports `HELD_CONSOLE` from here rather than restating `0`. A second
`const` there would be a third copy of the vocabulary — the one
`vocabulary-sync` does not compare, on the path taken when no store is present.

The console mint (`start_console_service`) moves here with its server spawn:
minting a capability and declaring it are the same event, and splitting them
across two files is how the two got out of step in the first place.

### 4. The order is an ABI, so a gate compares the two copies

The packer writes `slots[i] = k`, and `k`'s meaning lives in `authority.rs`.
That is one fact in two files, which this project has already got wrong twice
(the oracle-marker list, the `MAX_TASKS` census). `scripts/check/vocabulary-sync.sh`
extracts the `HELD_*` names and indices from both and fails on any disagreement
— the same derive-from-source technique `product-image.sh` uses, for the same
reason.

## Alternatives

| Option                                              | Why not                                                                                                                                                          |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Keep pushing minted caps into an array              | The shift bug above. It is silent, and silence is the argument                                                                                                   |
| A fixed-size `[Option<CapId>; N]` with no `declare` | Positions would be stable, but nothing names them, a vacancy prints as `None`, and two services could claim one index with no `Duplicate` refusal                |
| Index by name at bind time (`slots: [&str; 4]`)     | Names in the store are squattable and cost 16 bytes each in a 16 KiB window; the index discipline of ADR-0021 is stronger _because_ it is chosen at compose time |
| One `Refusal` variant per authority kind            | Breaks the `validate`/`bind` partition; the inner `BindError` already carries the detail                                                                         |

## Consequences

- `bind`'s signature changes, so every caller and every test moves in one commit.
- The boot log gains one line per declared position. That is the point: the
  vocabulary becomes visible on the wire, and a vacancy is a console line rather
  than a wrong grant.
- `MAX_HELD = 8` against `MAX_SLOTS = 4` per task: the vocabulary may be longer
  than any single agent can hold, which is correct — the ceiling on what one
  agent may reach stays where ADR-0017 put it.
- This ADR grants no new authority. It is the vocabulary; the words come next
  (device windows, then services on endpoints, then what the product binds — each
  its own ADR).

## The gate that would catch this ADR's reversal

`make product-boot-check` asserts `authority: 0 console ok` and refuses
`VACANT`; go back to a pushed array and the vacancy test in
`crates/kernel-core/src/held.rs` fails on the first hole. `vocabulary-sync`
fails the moment the packer and the kernel disagree about what index 1 means.
The reversal with no gate — nobody ever declaring a second position — is what
the roadmap rows are for.

## Evidence

| Level | What                                                                                                                                                                                                                                                            |
| ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Host  | A vacancy at 0 does not move 1; `Duplicate`/`Full`/`OutOfRange`/`AlreadyProvided`; `HeldVacant` and `NoSuchCapability` asserted as **distinct** refusals; the existing boundary test (`the_last_held_index_binds_and_the_next_one_does_not`) ported to `Option` |
| QEMU  | `authority: 0 console ok` in `make product-boot-check`; `VACANT` among the negatives; the store agents still load and run                                                                                                                                       |
| HW    | Pi 4B stamp carrying the `authority:` block on silicon                                                                                                                                                                                                          |
