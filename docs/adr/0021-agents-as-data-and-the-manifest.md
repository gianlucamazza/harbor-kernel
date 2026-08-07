---
id: 0021
title: Agents become data described by a manifest, and authority becomes enumerable in one artefact
status: proposed
date: 2026-08-07
related: [0012, 0017, 0018]
---

# ADR-0021: Agents as data, and the manifest that grants their authority

## Acceptance status

**Proposed** (2026-08-07). Required before the loader milestone by
[ADR-0001](0001-multi-role-analysis.md): it moves where authority is decided,
which is the boundary this project exists to defend.

## Context

An agent today is machine code compiled into the kernel and a grant written as
**code** in `bootstrap::run`:

```rust
spawn_with_slots(demos::el0_ipc_receiver, &[Some(ch.recv), console_cap])
```

Three consequences, all measured rather than supposed.

**Nothing can run that was not built with the kernel.** `Agent::poke_user` copies
from a `&[u8]` the kernel already holds. There is no loader, no storage path, no
format.

**An agent gets one page of text.** `USER_VA_BASE` maps page 0 as `USER_RX` and
pages 1..3 as stack: 4 KiB of code, 12 KiB of stack. The oracle's agents fit
because they are hand-written machine code; nothing compiled from a language
will.

**The grants are not enumerable.** `make product-builds` reported the shape of
this in one line: without the `oracle` feature, **88 items are unreachable** —
`sched::spawn`, `ipc::send`, `AddressSpace`, `task_trampoline`. The product
creates no task and sends no message, because the only creator is the test
scaffolding. `SECURITY.md` says a threat model needs authority to be enumerable;
today it is enumerable only by reading a function that also prints to the
console.

### One design that looked obvious and is wrong

Append the agents to `kernel8.img` and have the kernel read them from the end of
its own image. Measured: the image ends at `0x944e0`, and the linker script
places `__pagetables_start`, the exception stack, the kernel stack and
`__heap_start` from there on — `objcopy -O binary` omits them because they hold
no data. A blob appended to the file lands **inside `.bss` and the page-table
arena**, and boot overwrites it. Recorded because it is the first idea anyone
has.

## Decision

**1. An agent is data: a flat image plus a manifest entry.**

The flat image is bytes, not a Rust `fn`. `USER_VA_BASE` is fixed and text is
mapped at offset 0, so a program linked for that address needs no relocation and
no ELF parser — a parser is attack surface and image size bought for nothing
today. The manifest carries entry offset and sizes, so ELF can arrive later
without the manifest changing shape.

**2. The manifest is a binding, not a mint.**

An entry says: _this image, this many text pages, this many stack pages, these
device grants, and slot `n` holds capability `k` — where `k` names a channel the
loader already holds._ It cannot conjure authority. The kernel never reads a
capability out of the manifest; it reads an _index_ into what the creator built,
exactly as `Tcb.caps` is an index into what the creator granted
([ADR-0017](0017-el0-capability-abi.md) §2). The same structural argument, one
level up: a manifest that cannot name a capability outside the loader's own table
cannot escalate, and that is a property of the shape rather than of a check.

**3. The source of the data is separable, and step one is the kernel's own
`.rodata`.**

Embedded with `include_bytes!`. That is deliberately _not_ a filesystem: reading
from FAT needs an SD block driver, and by this project's own doctrine a block
driver is a driver-agent, which needs the loader. The circle is broken by not
entering it.

What this buys is the part that matters and is independent of where bytes come
from: **every grant in the machine is one table**, and the loader is one loop
over it. When the source later becomes a FAT file or a serial upload, the
manifest and the grant logic do not change — only what fills the array.

**4. The manifest starts as a Rust `const` table, and this ADR names when that
stops being enough.**

Compile-time, type-checked, zero parsing, zero attack surface. It becomes a byte
format the day the source stops being the image, because at that point the bytes
are input rather than source — and input from outside the image is the first
moment a parser is worth its risk. Not before.

**5. The user window is sized by the manifest.**

Text pages and stack pages become per-agent numbers instead of the fixed
`USER_STACK_PAGES = 4` geometry. `AddressSpace` already owns a
`FrameLedger<MAX_AS_FRAMES>` with `MAX_AS_FRAMES = 256`, so an agent may reach
1 MiB before that ceiling binds, against a 512-frame pool. Loader and geometry
are one decision, not two: a loader that can only place 4 KiB has not changed
what the system can run.

**6. The trust model does not change, and this ADR refuses to imply that it
does.**

The manifest lives in the image, so it is exactly as trusted as the code it
replaces. Nothing is sandboxed that was not sandboxed before. What changes is
that the trust is **named and in one place** instead of spread across a boot
function. That is the honest claim, and the temptation is to make a larger one.

## Consequences

### Positive

- Authority becomes enumerable in a single artefact — the property
  `SECURITY.md` needs and cannot currently state.
- The 88 unreachable items get a product-path caller: the loader creates agents
  because the manifest says so, not because a demo does.
- An agent can be written in a language, because its text is no longer bounded
  by one page.
- The oracle's agents become manifest entries like any other, so the boot check
  stops being the only thing that knows how to create one.

### Negative / debt

- **A manifest in the image is not a manifest from outside it.** Everything that
  makes this interesting as an _operating system_ — an agent the operator
  chooses, signature checking, a per-agent identity — begins only when the
  source changes, and this ADR deliberately does not start it.
- **No revocation, still.** The manifest grants at creation, and
  [ADR-0017](0017-el0-capability-abi.md)'s "grants happen at creation, nothing
  delegates at runtime" is untouched. A manifest makes the static topology
  legible; it does not make it dynamic.
- **A larger user window costs frames.** The pool is 512 frames; an agent asking
  for 64 pages takes an eighth of it. Refusal must be a reported error, not a
  panic — the frame allocator already returns `Option`.
- **The flat format has no version field until it is a byte format.** A `const`
  table changes with the code that reads it, which is the one advantage of not
  having parsed anything.

### Gates that catch reversal

| Reversal                                                  | Gate                                                                                                                                                    |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A grant written as code again, outside the manifest       | `make product-builds` — a `spawn_with_slots` outside the loader is scaffolding by definition, and the product image must not contain it                 |
| The manifest naming a capability the loader does not hold | Host test in `kernel-core`: the binding is index-into-table arithmetic, the same shape `cap::from_slot` already has and the bounded model already walks |
| An agent image larger than the window it declared         | `AddressSpace::poke_user` already bounds against `UserWindow::bound_text_write`; the manifest's page count becomes that window's size                   |
| Text growing back to one page by accident                 | `boot-check`: an agent whose image exceeds 4 KiB is in the oracle, so the assertion is that it ran                                                      |

## Alternatives rejected

| Alternative                                              | Why not                                                                                                                                                           |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Append the blob after `kernel8.img`                      | Measured to land inside `.bss` and the page-table arena — boot overwrites it. The image's end is not free space                                                   |
| ELF subset now                                           | A parser is attack surface and code size, bought for a relocation the fixed `USER_VA_BASE` does not need. The manifest carries what ELF headers would say         |
| FAT on the boot partition first                          | Needs an SD block driver; by ADR-0013's logic that belongs in an agent; an agent needs the loader. Entering the circle to leave it                                |
| Manifest as a byte format from the start                 | A parser with no input from outside the image is risk with no reader. It becomes right the moment the source moves, and this ADR says so rather than guessing now |
| Keep grants in `bootstrap` and just raise the page count | Fixes the 4 KiB and leaves authority spread across a boot function. The page count is the smaller half of the problem                                             |
