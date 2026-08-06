---
id: 0019
title: The last static mut becomes an atomic — a premise two ADRs inherited was false
status: accepted
date: 2026-08-06
accepted: 2026-08-07
related: [0016, 0017]
---

# ADR-0019: No `static mut`, including the one the assembly reaches

## Acceptance status

**Accepted** (2026-08-07), as proposed — the decision below is unchanged from
the version the project owner took. Landed the same day: `CURRENT_EL0` is an
`AtomicPtr`, `make no-static-mut` is a `make check` prerequisite, and rule 7
of [`architecture.md`](../architecture.md) no longer has an exception.

Successor to the second decision of
[ADR-0016](0016-el0-session-protocol.md) and to the paragraph of
[ADR-0017](0017-el0-capability-abi.md) §1 that repeats it. Both are immutable —
0016 as `superseded`, 0017 as `accepted` — so this exists rather than an edit.

## Context

Rule 7 of [`architecture.md`](../architecture.md) is unambiguous:

> State shared between the IRQ path and the main loop uses
> `core::sync::atomic` — never `static mut`.

`arch::el0::CURRENT_EL0` is written by the scheduler on every switch and read by
the exception path — `vectors.s` dereferences it on every lower-EL exception. It
is precisely the state rule 7 names, and it is a `static mut`.

It was permitted by an argument, stated in ADR-0016 §2 and repeated verbatim in
ADR-0017 §1:

> The assembly reaches them by symbol name (`adrp`/`add`). A `SyncCell` has no
> linker-visible name to load.

**The argument is false**, and this ADR exists because it was inherited twice
without anyone testing it. What `adrp`/`add` needs is a symbol at a known
address. `#[unsafe(no_mangle)]` gives that to _any_ `static`, mutable or not,
and `AtomicPtr<T>` is `#[repr(transparent)]` over `UnsafeCell<*mut T>` — so the
symbol's address **is** the pointer's address by a language guarantee rather
than by a layout that happens to work.

Measured before this ADR was written, not after:

```
static mut CURRENT_EL0: *mut El0Session       →  00000000000970c0 B CURRENT_EL0
static CURRENT_EL0: AtomicPtr<El0Session>     →  00000000000944e0 B CURRENT_EL0
```

Same symbol, same section, assembly unchanged, `boot-check: clean`. The address
differs only because the rest of the image moved.

One nuance the original argument was half-right about: a `SyncCell<T>` as
declared in `src/sync.rs` is _not_ `#[repr(transparent)]`, so its field offset is
not guaranteed even with one field. For `SyncCell` the objection would have had
to be "add `#[repr(transparent)]` first", not "it has no name". An atomic needs
neither, and says more.

## Decision

**1. `CURRENT_EL0` becomes `AtomicPtr<El0Session>`.** No `static mut` remains in
the kernel. `publish` and `published` stop being `unsafe` to _perform_ — the
pointer's dereference stays `unsafe`, which is the operation that actually
carries the obligation.

**2. The memory ordering is stated rather than assumed.** `Release` on publish,
`Acquire` on read. On one core this is free; what it buys is that the ordering
between "the TCB's session fields are initialised" and "the pointer is visible"
is written down where the next reader looks, instead of being a property of
single-core execution that a second core would silently remove.

**3. Rule 7 gains no exception.** It reads as an absolute today and was not one;
after this it is.

**4. The false premise is retracted here, not silently dropped.** ADR-0016 and
ADR-0017 keep their text — they are immutable — and this ADR is what a reader
following the `related` links finds.

## Consequences

### Positive

- The kernel has no `static mut`. `src/sync.rs`'s opening argument — that a
  `static mut` states its contract in a comment, and a comment cannot be checked
  — now applies to the whole tree with no "except one".
- Two `unsafe` blocks disappear, and the remaining one is the dereference, which
  is where the danger actually is.
- The publication ordering is a declaration instead of a single-core accident.

### Negative / debt

- **`AtomicPtr` promises more than one core delivers.** Nothing here is
  multi-core, and an atomic pointer can read as though the design were SMP-ready.
  It is not: `El0Session` behind it is `&mut`-aliased under an interrupt mask,
  and a second core needs a lock, not an atomic. The type is chosen for its
  layout guarantee and its ordering vocabulary, not as a concurrency claim.
- **The assembly still hard-codes that this symbol is a pointer.** Nothing
  compares `vectors.s`'s `ldr x16, [x16]` against the Rust type. The offsets
  _inside_ the session are derived (`.equ` from `offset_of!`); the fact that the
  symbol holds a pointer is not, and no gate covers it.

### Gates that catch reversal

| Reversal                                             | Gate                                                                                 |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------ |
| A `static mut` reappears anywhere in `src/`          | `make no-static-mut` — a new grep gate, seen red against the tree as it stands today |
| The published pointer stops being the current task's | the `CURRENT_EL0` assertion from ADR-0017 §1, already seen red                       |
| The symbol stops being loadable by name              | link failure — `vectors.s` names it                                                  |

## Alternatives rejected

| Alternative                                                     | Why not                                                                                                                                                             |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Leave it, cite ADR-0016                                         | The citation is to an argument that is false. Leaving it means the tree contains one violation of its own rule 7 whose only defence is a sentence nobody re-checked |
| `SyncCell` with `#[repr(transparent)]`                          | Works, and says less. It asserts `Sync` by hand where an atomic _is_ `Sync`, and it leaves the publication ordering unstated                                        |
| A `static` behind a raw-pointer newtype with `unsafe impl Send` | The same, plus a type that exists only to satisfy a bound                                                                                                           |
| Pass the session pointer to the assembly in a register          | `vectors.s` is entered by hardware on an exception. There is no caller to pass anything                                                                             |
