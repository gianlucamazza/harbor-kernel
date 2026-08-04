# Verification

What is checked, by what, and — the part that matters — what each check cannot
see. A gate whose blind spots are undocumented gets trusted for things it never
covered.

## The layers

| Layer                                     | Runs                                    | Covers                                                                                                                 | Blind to                                                                                       |
| ----------------------------------------- | --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Host unit tests (`make test`)             | `cargo test -p kernel-core`             | Register encodings, allocator arithmetic, GIC index maths, region splitting, the SPSC ring                             | Anything that touches hardware, and any _use_ of these functions                               |
| Miri (`make miri`)                        | Interprets the host tests               | Aliasing, provenance and data races in the crate's only `unsafe` — the ring's `UnsafeCell` buffer and `Sync` assertion | The kernel crate's `unsafe`, which touches MMIO and system registers and cannot be interpreted |
| Bring-up build (`make bringup-builds`)    | Compiles and lints `--features bringup` | A configuration nothing else builds, and the one you reach for when the board will not talk                            | Anything the gates do not _run_ — it compiles, it is not executed                              |
| No-SIMD guard (`make no-simd`)            | Disassembles the linked image           | A build that silently regains FP/SIMD                                                                                  | FP that never reaches the image                                                                |
| Pre-MMU path (`make no-early-exclusives`) | Disassembles `_start` and its callees   | Atomic read-modify-write before translation is on, the path growing, and any indirect branch on it                     | Nothing on that path: an edge it cannot follow is refused rather than skipped                  |
| QEMU boot (`make boot-check`)             | Boots the image, asserts on the log     | MMU activation, allocator reclaim, timer IRQ, WFI idle, unhandled interrupts, panics                                   | **Memory attributes.** Also cache behaviour, real clocks, firmware state                       |
| Doc claims (`make doc-claims`)            | Compares README against the Makefile    | The two README claims a machine can settle: the `make check` gate list and the host test count                         | Every other sentence in the docs, which is prose and stays prose                               |
| Layering (`make layering`)                | Every `crate::` import edge in `src/`   | The rules in `architecture.md`: drivers never know the board, arch never names a driver, `exception` reaches only `irq` | Coupling that is not an import — a shared constant, an agreed register value, a naming convention |
| Hardware                                  | A Pi 4B on a serial console             | Everything above, for real                                                                                             | Only what you actually boot and look at                                                        |

`make check` runs every layer above except the hardware one, and is deliberately
a superset of CI: each CI job has a target here, so a green locally predicts a
green remotely. That claim is load-bearing and easy to break — it was false for
part of one day, when a Miri job was added to CI without adding it to
`make check`. A verification claim that is false is worse than one that is
absent, because someone relies on it.

Two escape hatches, both explicit:

| Situation       | Behaviour                                              |
| --------------- | ------------------------------------------------------ |
| QEMU missing    | `boot-check` **fails**; `ALLOW_BOOT_SKIP=1` to opt out |
| nightly missing | `miri` skips with a message                            |

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

## TLB maintenance: encoding vs necessity

`mmu::map` and `mmu::unmap` issue `tlbi vaae1is` per page, or `vmalle1` past the
threshold, and the operand encoding is unit-tested (`tlbi_plan`, and the
mutation that dropped the `>> 12`). Hardware has exercised the per-page branch
for real on `map` — the DTB is 15 pages, so a live boot takes the branch QEMU
never does, since its 2 MiB region always resolves to `Everything`.

**invalid→valid (`map`):** an invalid entry is not architecturally permitted to
be cached, so dropping the invalidation would very likely change nothing
observable. Encoding is covered; necessity is not.

**valid→invalid (`unmap`):** a stale TLB entry keeps the old translation. That
is the first path where maintenance is load-bearing. It lands with M3 task-stack
guards (ADR-0006). Production boots exercise unmap+remap in `heap_check` (QEMU
gated). A deliberate **task-stack guard write** is behind `--features bringup`
(`selftest::probe_task_stack_guard`): it must panic with a translation fault.
Record the ESR/FAR line under [M3 hardware evidence](#m3-cooperative-tasks-hardware)
when silicon is available. Until that row exists, treat TLB *necessity* as
construction + QEMU smoke, not silicon-closed.

## Protections are only verified when you have seen them fire

W^X and the guard page are claims about what _fails_. A map that reports itself
active proves nothing about enforcement. Both were checked by temporarily
adding a deliberate fault to `bootstrap::run` and booting on hardware:

| Probe                        | ESR          | Decoded                                                        | FAR       | Run on    |
| ---------------------------- | ------------ | -------------------------------------------------------------- | --------- | --------- |
| Write to `.text` (`0x80000`) | `0x9600004F` | EC 0x25 data abort, DFSC `0b001111` permission fault L3, WnR=1 | `0x80000` | hardware  |
| Write to the guard page      | `0x96000047` | EC 0x25 data abort, DFSC `0b000111` translation fault L3       | `0xa1000` | hardware  |
| Kernel stack overflow        | `0x96000047` | EC 0x25 data abort, DFSC `0b000111` translation fault L3       | `0xa1ff8` | hardware  |

The translation fault is the one to insist on for the guard page: a
_permission_ fault there would mean the page is mapped but protected, and a
stack that overflowed by reading would not be caught.

All three rows are hardware runs against the current tree. The W^X row was taken
before the stack split and not re-run: `.text` and `.rodata` were not touched by
it, and its ESR does not depend on an address that moved.

The probes are not in the tree — a deliberate fault is a dead board. Re-run
them by hand after changing `link.ld` or the region list in `mm::layout`. This
table is the only copy: it used to be duplicated in `mmu.md`, and both copies
went stale together the moment the layout moved.

## M3 cooperative tasks (hardware)

**Status: open.** QEMU is closed (`boot-check` asserts interleaved `task-a` /
`task-b`, spawn, unmap smoke, ticks). Silicon is not yet recorded in this tree.

| Check | How | Evidence |
| --- | --- | --- |
| Interleaved yield | Production image on Pi 4B serial | *pending* — expect `task-a`/`task-b` lines and `CNTFRQ=54000000` |
| Task-stack guard fault | `cargo build --release --features bringup`, flash, capture panic | *pending* — expect `PROBE: writing to task stack guard at …` then `ESR=…` DFSC translation, FAR in guard |
| Review | [2026-08-04-m3-incremental.md](reviews/2026-08-04-m3-incremental.md) | desk pass done; HW checklist open |

When both captures exist, paste them here and flip the M3 row in
`architecture.md` to `done (HW)`.

### Lab procedure (task guard)

```bash
cargo build --release --features bringup
llvm-objcopy -O binary target/aarch64-unknown-none-softfloat/release/harbor-kernel \
  target/aarch64-unknown-none-softfloat/release/kernel8.img
make deploy SD_MOUNT=/run/media/$USER/bootfs   # adjust mount
make serial
# Expect selftest GIC gates, then PROBE line, then PANIC with ESR/FAR.
# Re-flash a production image (no bringup) afterwards — bringup panics by design.
```

## Hardware evidence: stack split (closed)

The stack split (`SP_EL0` for the kernel, `SP_EL1` for exceptions) changed the
boot sequence and the vector group the hardware enters through — both in the
category this project has already been burned by, where emulation agrees and
silicon does not. **Boot, overflow probe, and guard-page write are all closed
on hardware**; this section is the evidence, not an open checklist.

**Boot.** On a Pi 4B, 2026-08-04:

```
MMU on  (W^X, guard page at 0xa1000, 40960 B of table arena left)
DTB mapped: 61440 bytes at 0x2eff1000
heap: 67108864 bytes free after drop (fully reclaimed), 1 fragments
ticks=10 … ticks=70
```

`CNTFRQ=54000000` says this is silicon, not TCG; `0xa1000` says it is the split
layout and not a stale card. Timer IRQs arrive, which is the part worth
insisting on: they can only arrive through the **EL1t** vector entries, so the
vector group moved correctly and the hardware really does switch to `SP_EL1`.

**Overflow probe.** On the same board, a small-frame recursion into the guard
page:

```
PROBE: overflowing the kernel stack
  ESR=0x0000000096000047   ELR=0x00000000000812bc
  SPSR=0x0000000060000344  FAR=0x00000000000a1ff8
```

`FAR=0xa1ff8` is the top of the guard page: the handler stopped at the first
byte that faulted instead of walking down through it. The `SPSR` is independent
evidence for the same thing — `M[3:0] = 0b0100` is EL1t, so the interrupted
context was running on `SP_EL0`. Before the split the same probe recorded
`SPSR=0x3c5`, `M[3:0] = 0b0101`, EL1h.

**Guard-page write probe**, at the address the split moved it to:

```
PROBE: writing to the guard page at 0xa1000
  ESR=0x0000000096000047  FAR=0x00000000000a1000
```

DFSC `0b000111` is a translation fault, not a permission fault, which is the
property that matters: an unmapped page catches an overflowing *read* too.

It took two runs. The first was captured while a stale monitor still held the
port, and the two readers split the stream — `CNTFRQ=5400096000047` is one line
of each. The bytes could have been stitched back together from the two logs, and
the answer would have been right, but a reconstructed stream is what produced a
wrong conclusion earlier in this project. The probe was re-run with one reader
instead.

The W^X probe needs no re-run: `.text` and `.rodata` were not touched by the
split, and its recorded ESR does not depend on an address that moved.

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

| Check                                                            | Mutation                                    | Observed                                                                                                                             |
| ---------------------------------------------------------------- | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| PL011 divisors, bump alignment, `TCR.EPD1`, descriptor alignment | original implementations                    | 10 red tests before the fixes                                                                                                        |
| SPSC ring ordering                                               | publish `head` before writing the slot      | `out of sequence at 8572`                                                                                                            |
| Allocator coalescing                                             | drop the backward merge                     | `arena must be whole again`, `churn left the arena fragmented`                                                                       |
| L3 descriptor encoding                                           | encode an L3 leaf as a block                | `L3 leaf must be 0b11`                                                                                                               |
| No-SIMD guard                                                    | the pre-softfloat image                     | `dup v0.4h` in `memset`                                                                                                              |
| Pre-MMU path                                                     | a Rust `fetch_add` called from `_start`     | named the symbol and explained the fix                                                                                               |
| QEMU boot check                                                  | remove `irq::enable(TIMER_IRQ)`             | missing tick reports                                                                                                                 |
| Trap frame coupling                                              | grow `TrapFrame` by 16 bytes                | the stub's reservation moved `0x110` → `0x120`                                                                                       |
| Blob integrity                                                   | corrupt an expected hash                    | refused to install, exit 1                                                                                                           |
| Miri                                                             | publish `head` before writing the slot      | `Undefined Behavior: Data race detected between (1) non-atomic write and (2) non-atomic read`                                        |
| `mmu::map` overwrite refusal                                     | map the same region twice                   | `AlreadyMapped(0x8000000)` instead of a silent replacement                                                                           |
| Bring-up build gate                                              | rename a function used only there           | `make bringup-builds` red, `E0425`                                                                                                   |
| Layout validator                                                 | `GUARD_PAGE_SIZE = 0` in `link.ld`          | `LAYOUT INVALID: GuardIneffective` — and the first attempt at that check passed, which is how the linker-symbol fold below was found |
| Refusal to boot unprotected                                      | make `mmu::activate` return `OutOfTables`   | `BOOT REFUSED: could not map planted failure` and then nothing — no heap line, no ticks, no console loop                                    |
| Pre-MMU path, indirect branch                                    | reach the gate through `blr x9`             | `indirect branch in _start: its target is not derivable` — the call graph the check walks had a hole                                 |
| Layering rules                                                   | `drivers` imports `bsp`; `arch` imports `drivers`; `exception` imports `drivers` | one line naming the module and the edge, for each of the three rules separately                                    |
| RX bytes dropped                                                 | shrink the ring to 4 bytes and paste 60     | `console: DROPPED 57 received bytes (ring full)`, where before the loss was invisible                                                |
| Exception stack (`SP_EL1`)                                       | run the same overflow on the pre-split tree | `FAR=0x9c000`, the guard's **bottom**, against `0xa1ff8`, its **top** — the handler had walked the whole page and landed below it    |
| Exception-stack guard page                                       | zero-length exception guard in `Boundaries` | `GuardIneffective` — validation is written once over both stacks, and this is what keeps that true                                   |
| Double-free refusal (the mark)                                   | stop consulting the allocated bit           | one test red — the one where alignment leaves the back-pointer intact, which is the only case the sentinel cannot catch              |
| Double free through the real allocator                           | free the same pointer twice in `console_loop`      | `heap: REFUSED 1 invalid frees`, boot check red, and the heap still `fully reclaimed`                                                |
| Doc claims (test count)                                          | restore the stale `54 host unit tests`      | `README claims 54 host unit tests, there are 77` — the exact drift it was written for                                                |
| Doc claims (gate list)                                           | drop `bringup-builds` from the README       | printed both lists side by side; this is F27, which had already happened twice for real                                              |
| TLBI operand shift                                               | drop the `>> 12`                            | three tests red — the operand became the address, invalidating a different page                                                      |
| Runtime mapping (`mmu::map`)                                     | skip the call, keep the read                | `ESR=0x96000006` level-2 translation fault at the blob address; with the call, `0xd00dfeed`                                          |

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
which states what is actually meant — _the number the linker chose_ — and which
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
