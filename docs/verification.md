# Verification

What is checked, by what, and — the part that matters — what each check cannot
see. A gate whose blind spots are undocumented gets trusted for things it never
covered.

## The layers

| Layer                                     | Runs                                  | Covers                                                                                     | Blind to                                                                 |
| ----------------------------------------- | ------------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------ |
| Host unit tests (`make test`)             | `cargo test -p kernel-core`           | Register encodings, allocator arithmetic, GIC index maths, region splitting, the SPSC ring | Anything that touches hardware, and any _use_ of these functions         |
| Miri (`make miri`)                        | Interprets the host tests             | Aliasing, provenance and data races in the crate's only `unsafe` — the ring's `UnsafeCell` buffer and `Sync` assertion | The kernel crate's 51 `unsafe` sites, which touch MMIO and cannot be interpreted |
| Bring-up build (`make bringup-builds`)    | Compiles and lints `--features bringup` | A configuration nothing else builds, and the one you reach for when the board will not talk | Anything the gates do not *run* — it compiles, it is not executed |
| No-SIMD guard (`make no-simd`)            | Disassembles the linked image         | A build that silently regains FP/SIMD                                                      | FP that never reaches the image                                          |
| Pre-MMU path (`make no-early-exclusives`) | Disassembles `_start` and its callees | Atomic read-modify-write before translation is on, and the path growing                    | Exclusives reached indirectly through a function pointer                 |
| QEMU boot (`make boot-check`)             | Boots the image, asserts on the log   | MMU activation, allocator reclaim, timer IRQ, WFI idle, unhandled interrupts, panics       | **Memory attributes.** Also cache behaviour, real clocks, firmware state |
| Hardware                                  | A Pi 4B on a serial console           | Everything above, for real                                                                 | Only what you actually boot and look at                                  |

`make check` runs every layer above except the hardware one, and is deliberately
a superset of CI: each CI job has a target here, so a green locally predicts a
green remotely. That claim is load-bearing and easy to break — it was false for
part of one day, when a Miri job was added to CI without adding it to
`make check`. A verification claim that is false is worse than one that is
absent, because someone relies on it.

Two escape hatches, both explicit:

| Situation | Behaviour |
| --------- | --------- |
| QEMU missing | `boot-check` **fails**; `ALLOW_BOOT_SKIP=1` to opt out |
| nightly missing | `miri` skips with a message |

Skipping is never silent. A check that passes when it cannot run reports
coverage it does not have, and "skipped" scrolls past in a log that ends in a
green tick.

## What emulation cannot catch, with the example that proved it

QEMU's TCG implements load/store-exclusive with a global monitor that **ignores
memory attributes**. On a Cortex-A72 with translation off, every access is
Device-nGnRnE, where the `LDXR`/`STXR` pair behind `AtomicBool::swap` makes no
forward progress: the retry loop spins forever.

A kernel with an `AtomicBool::swap` in `console::acquire` — the first statement
of `bootstrap::run` — therefore booted perfectly under QEMU and hung on the
board with no output and no fault. The ACT LED lit while the firmware read the
card and went out, which is the signature of a _successful_ load, so even the
board's own diagnostics pointed away from the kernel.

The fix was not to remember the rule. `boot.s` now enables a compile-time
identity map before any Rust runs, so the window does not exist, and
`scripts/check-pre-mmu-path.sh` fails the build if anything re-enters it.

**Rule of thumb:** if a change concerns memory attributes, cache maintenance,
exclusive access, or the state the firmware leaves behind, a green QEMU boot is
not evidence.

## Protections are only verified when you have seen them fire

W^X and the guard page are claims about what _fails_. A map that reports itself
active proves nothing about enforcement. Both were checked by temporarily
adding a deliberate fault to `bootstrap::run` and booting on hardware:

| Probe                        | ESR          | Decoded                                                        | FAR       |
| ---------------------------- | ------------ | -------------------------------------------------------------- | --------- |
| Write to `.text` (`0x80000`) | `0x9600004F` | EC 0x25 data abort, DFSC `0b001111` permission fault L3, WnR=1 | `0x80000` |
| Write to the guard page      | `0x96000047` | EC 0x25 data abort, DFSC `0b000111` translation fault L3       | `0x9a000` |

The translation fault is the one to insist on for the guard page: a
_permission_ fault there would mean the page is mapped but protected, and a
stack that overflowed by reading would not be caught.

The probes are not in the tree — a deliberate fault is a dead board. Re-run
them by hand after changing `link.ld` or the region list in `mm::layout`.

## Bring-up gates

`cargo build --features bringup` adds masked CNTP / HPPIR / IAR gates that
reproduce the sequence used to debug the interrupt path. They reach for raw GIC
registers, which is why they are not in a production image.

Worth re-running on hardware after anything that changes the memory regime, and
after a firmware bump — the GIC group configuration is inherited from
`start4.elf` (see [`blobs.md`](blobs.md)). Last verified on a Pi 4B with the
early MMU active:

```
selftest: soft_ticks=3      CNTP fires with IRQs masked
gate: HPPIR=30 ok           the distributor reports the timer PPI pending
inject: IAR=0x1e id=30      a manual claim returns the timer id
inject: ticks 0 -> 2        and advances the counter
selftest: OK
```

A failing gate drops into a polled console rather than going quiet, so failure
is observable too.

## Checks that have been seen to fail

A test that has never failed has not been shown to test anything. Each of these
was confirmed by breaking the thing on purpose and watching the gate go red:

| Check                                                            | Mutation                                | Observed                                                       |
| ---------------------------------------------------------------- | --------------------------------------- | -------------------------------------------------------------- |
| PL011 divisors, bump alignment, `TCR.EPD1`, descriptor alignment | original implementations                | 10 red tests before the fixes                                  |
| SPSC ring ordering                                               | publish `head` before writing the slot  | `out of sequence at 8572`                                      |
| Allocator coalescing                                             | drop the backward merge                 | `arena must be whole again`, `churn left the arena fragmented` |
| L3 descriptor encoding                                           | encode an L3 leaf as a block            | `L3 leaf must be 0b11`                                         |
| No-SIMD guard                                                    | the pre-softfloat image                 | `dup v0.4h` in `memset`                                        |
| Pre-MMU path                                                     | a Rust `fetch_add` called from `_start` | named the symbol and explained the fix                         |
| QEMU boot check                                                  | remove `irq::enable(TIMER_IRQ)`         | missing tick reports                                           |
| Trap frame coupling                                              | grow `TrapFrame` by 16 bytes            | the stub's reservation moved `0x110` → `0x120`                 |
| Blob integrity                                                   | corrupt an expected hash                | refused to install, exit 1                                     |
| Miri                                                             | publish `head` before writing the slot  | `Undefined Behavior: Data race detected between (1) non-atomic write and (2) non-atomic read` |
| `mmu::map` overwrite refusal                                     | map the same region twice              | `AlreadyMapped(0x8000000)` instead of a silent replacement |
| Bring-up build gate                                              | rename a function used only there      | `make bringup-builds` red, `E0425` |
| Layout validator                                                 | `GUARD_PAGE_SIZE = 0` in `link.ld`      | `LAYOUT INVALID: GuardIneffective` — and the first attempt at that check passed, which is how the linker-symbol fold below was found |
| TLBI operand shift                                               | drop the `>> 12`                       | three tests red — the operand became the address, invalidating a different page |
| Runtime mapping (`mmu::map`)                                     | skip the call, keep the read           | `ESR=0x96000006` level-2 translation fault at the blob address; with the call, `0xd00dfeed` |

## What Miri adds over the two-thread test

Both catch the same mutation, and they say different things. Publishing `head`
before writing the slot makes the native test report `out of sequence at 8572`
— a symptom, found by sampling one interleaving out of many. Miri names the
cause: a data race between a non-atomic write and a non-atomic read. One tells
you a value was wrong; the other tells you the program is undefined.

Miri interprets rather than executes, at roughly 100x the cost, so the two
long-running tests carry `#[cfg(miri)]` bounds: 512 items instead of 200 000,
150 churn rounds instead of 2000. The shape of these tests is what finds bugs,
not the volume.

It runs on nightly, which is why it is a separate CI job and not part of
`make check` — the toolchain pin is deliberately stable, and a nightly
requirement must not leak into the gate everything else runs under.

## Two linker symbols can share an address; the compiler assumes they cannot

`__guard_end` and `__stack_bottom` name the same address by construction — the
guard page ends exactly where the stack begins. Declared as `static X: u8`,
each claims to be a one-byte object, and LLVM correctly derives from that claim
that distinct objects occupy distinct storage. So `guard_end == stack_bottom`
folded to `false`, and the layout validator rejected a perfectly good map.

Casting to an integer does not help — the fold happens on the `ptrtoint`
operands. `core::hint::black_box` suppresses it and is the wrong tool: its own
documentation says the behaviour is unspecified and must not be relied on for
correctness. The addresses are now materialised with an `asm!` `sym` operand,
which states what is actually meant — *the number the linker chose* — and which
the compiler cannot fold because it cannot see through it.

The symptom is worth remembering: every address printed correctly, while a
comparison built from those same addresses came out wrong. Deduction kept
saying the code was right; printing the comparison itself is what found it.

## Serial capture

One reader per port. Two `cat /dev/ttyUSB0` processes split the byte stream
between them, which looks like a kernel dropping output: lines truncated
mid-word and tick reports arriving at 30, 50, 70 instead of every 10. The
_regularity_ of the loss is the tell — a broken kernel does not drop bytes on a
schedule.

The USB-serial adapter can also back-feed the board through the GPIO pins. With
the Pi's own supply removed the red PWR LED stays lit, the SoC never fully
powers down, and the EEPROM does not restart: every "power cycle" after the
first is a no-op, and the board sits silent with a perfectly good card in it.
Do not wire the adapter's VCC line; if the back-feed persists through TX/RX,
unplug the adapter from USB as part of each cycle.
