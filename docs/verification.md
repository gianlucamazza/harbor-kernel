# Verification

What is checked, by what, and — the part that matters — what each check cannot
see. A gate whose blind spots are undocumented gets trusted for things it never
covered.

## The layers

| Layer                                     | Runs                                  | Covers                                                                                     | Blind to                                                                 |
| ----------------------------------------- | ------------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------ |
| Host unit tests (`make test`)             | `cargo test -p kernel-core`           | Register encodings, allocator arithmetic, GIC index maths, region splitting, the SPSC ring | Anything that touches hardware, and any _use_ of these functions         |
| No-SIMD guard (`make no-simd`)            | Disassembles the linked image         | A build that silently regains FP/SIMD                                                      | FP that never reaches the image                                          |
| Pre-MMU path (`make no-early-exclusives`) | Disassembles `_start` and its callees | Atomic read-modify-write before translation is on, and the path growing                    | Exclusives reached indirectly through a function pointer                 |
| QEMU boot (`make boot-check`)             | Boots the image, asserts on the log   | MMU activation, allocator reclaim, timer IRQ, WFI idle, unhandled interrupts, panics       | **Memory attributes.** Also cache behaviour, real clocks, firmware state |
| Hardware                                  | A Pi 4B on a serial console           | Everything above, for real                                                                 | Only what you actually boot and look at                                  |

`make check` runs all of the local layers and is deliberately a superset of CI,
so a green here predicts a green there.

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
