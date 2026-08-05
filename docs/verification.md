# Verification

What is checked, by what, and — the part that matters — what each check cannot
see. A gate whose blind spots are undocumented gets trusted for things it never
covered.

## The layers

| Layer                                     | Runs                                    | Covers                                                                                                                 | Blind to                                                                                       |
| ----------------------------------------- | --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Host unit tests (`make test`)             | `cargo test -p kernel-core`             | Register encodings (UART, SPI, RNG200, …), allocator arithmetic, GIC index maths, region splitting, the SPSC ring      | Anything that touches hardware, and any _use_ of these functions                               |
| Miri (`make miri`)                        | Interprets the host tests               | Aliasing, provenance and data races in the crate's only `unsafe` — the ring's `UnsafeCell` buffer and `Sync` assertion | The kernel crate's `unsafe`, which touches MMIO and system registers and cannot be interpreted |
| Bring-up build (`make bringup-builds`)    | Compiles and lints `--features bringup` | A configuration nothing else builds, and the one you reach for when the board will not talk                            | Anything the gates do not _run_ — it compiles, it is not executed                              |
| No-SIMD guard (`make no-simd`)            | Disassembles the linked image           | A build that silently regains FP/SIMD                                                                                  | FP that never reaches the image                                                                |
| Pre-MMU path (`make no-early-exclusives`) | Disassembles `_start` and its callees   | Atomic read-modify-write before translation is on, the path growing, and any indirect branch on it                     | Nothing on that path: an edge it cannot follow is refused rather than skipped                  |
| QEMU boot (`make boot-check`)             | Boots the image, asserts on the log     | MMU activation, allocator reclaim, timer IRQ, WFI idle, unhandled interrupts, panics                                   | **Memory attributes.** Also cache behaviour, real clocks, firmware state. RNG200 is not modelled on `raspi4b` — init reports `NotPresent` via `arch::probe`, not a successful FIFO read |
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
is the first path where maintenance is load-bearing. Production boots exercise
unmap+remap and a forced 2 MiB **block split** in `heap_check` (QEMU gated;
also seen on silicon). Task-stack guards use the same unmap path; a scheduled
overflow probe on hardware took a translation fault in the guard
([M3 evidence](#m3-cooperative-tasks-hardware)). That is strong evidence the
invalidation is *necessary* for guards; a deliberate “strip TLBI and re-run”
mutation is still optional if you want a pure TLB-only experiment.

## Protections are only verified when you have seen them fire

W^X and the guard page are claims about what _fails_. A map that reports itself
active proves nothing about enforcement. Both were checked by temporarily
adding a deliberate fault to `bootstrap::run` and booting on hardware:

| Probe                        | ESR          | Decoded                                                        | FAR       | Layout when run |
| ---------------------------- | ------------ | -------------------------------------------------------------- | --------- | --------------- |
| Write to `.text` (`0x80000`) | `0x9600004F` | EC 0x25 data abort, DFSC `0b001111` permission fault L3, WnR=1 | `0x80000` | any — `.text` starts at the image base |
| Write to the guard page      | `0x96000047` | EC 0x25 data abort, DFSC `0b000111` translation fault L3       | `0xa1000` | guard at `0xa1000`, pre-M3 |
| Kernel stack overflow        | `0x96000047` | EC 0x25 data abort, DFSC `0b000111` translation fault L3       | `0xa1ff8` | guard at `0xa1000`, pre-M3 |

The translation fault is the one to insist on for the guard page: a
_permission_ fault there would mean the page is mapped but protected, and a
stack that overflowed by reading would not be caught.

**These are dated observations, not current addresses.** The bootstrap guard has
since moved to `0xa2000` and then `0xa3000` — not because anything about it
changed, but because `.text` grew underneath it, which happens on any commit
that adds code. What each row asserts is the **ESR**, which does not depend on
where the guard sits; the `FAR` column is meaningful only against the layout
named beside it, and the boot line prints the guard's current address on every
boot.

That is why the addresses are not tracked: a doc gate that compared them to the
running binary would go red on commits that changed nothing it was meant to
protect. Re-run the two guard rows when the *mechanism* changes — a different
guard strategy, a different stack arrangement — not when the address moves.

The probes are not in the tree — a deliberate fault is a dead board. Re-run
them by hand after changing `link.ld` or the region list in `mm::layout`. This
table is the only copy: it used to be duplicated in `mmu.md`, and both copies
went stale together the moment the layout moved.

## M3 cooperative tasks (hardware)

| Check | Status | Evidence |
| --- | --- | --- |
| Interleaved yield + unmap smoke | **closed (HW)** | Pi 4B serial, 2026-08-04 — transcript below |
| Task-stack guard fault | **closed (HW)** | bringup image, 2026-08-05 — ESR table below |
| Review | desk done | [2026-08-04-m3-incremental.md](reviews/2026-08-04-m3-incremental.md) |

QEMU remains gated by `boot-check`. Both silicon rows above are closed: M3 may
be marked `done (HW)`.

### Boot + cooperative yield (closed)

Pi 4B, production image, CP2104 @ 115200, 2026-08-04. `CNTFRQ=54000000` is
silicon (TCG is 62.5 MHz). The guard sat at `0xa2000` in the image that was
flashed; it has moved since, with `.text` — see the probe table above on why
that is expected and not tracked.

```
Harbor: hello
EL1 · W^X map · heap · timer + UART RX IRQ · WFI idle
DTB at 0x2eff1f00
MMU on  (W^X, guard page at 0xa2000, 40960 B of table arena left)
DTB mapped: 61440 bytes at 0x2eff1000
heap remaining = 67108864 bytes
CNTFRQ=54000000 Hz  timer=10 Hz  PPI=30
IRQs enabled (timer + UART RX)
idle: WFI when no RX/tick work
heap: Box at 0xb3010, Vec of 1024 sums to 523776
heap: 67100624 bytes free while held, 2 fragments
heap: 67108864 bytes free after drop (fully reclaimed), 1 fragments
unmap: page at 0xb4000 fault-ready
unmap: remapped and freed
sched: spawned task-a
sched: spawned task-b
task-a 0
task-b 0
task-a 1
task-b 1
task-a 2
task-b 2
task-a 3
task-b 3
ticks=10
…
ticks=410
```

**Later production boot** (same board, post–block-split smoke, 2026-08-05) also
shows `split: page at 0x200000 split 1, remapped` and `arena: 1 splits, …`
before the interleaved tasks — matching the QEMU `boot-check` oracle.

No `irq: unhandled`, no `timer: MISSED`, no panic through several minutes of idle.

### Task-stack overflow guard (closed)

Pi 4B, `--features bringup`, CP2104 @ 115200, 2026-08-05. The probe is a
**scheduled task** that recurses while two peer task stacks are live; it prints
every range first so `FAR` is checked against peers, not deduced.

```
sched: spawned task-a
sched: spawned task-b
sched: spawned guard probe
arena: 0 splits, 9 tables free
task-a 0
task-b 0
PROBE: overflowing task 3 of 3 live stacks
PROBE: peer task 1 guard 0xb6000..0xb7000 stack 0xb7000..0xbb000
PROBE: peer task 2 guard 0xbc000..0xbd000 stack 0xbd000..0xc1000
PROBE: self task 3 guard 0xc2000..0xc3000 stack 0xc3000..0xc7000
PROBE: recursing until the guard faults

*** KERNEL PANIC ***
  ESR=0x0000000096000047
  ELR=0x0000000000083f64
  SPSR=0x0000000080000344
  FAR=0x00000000000c2ff8
```

| Field | Value | Meaning |
| --- | --- | --- |
| ESR | `0x96000047` | EC 0x25 data abort; DFSC `0b000111` **translation fault L3** |
| FAR | `0xc2ff8` | top of **self** guard `[0xc2000, 0xc3000)` |
| Peers | `0xb7…`, `0xbd…` | FAR is **outside** both peer stacks |

Same DFSC class as the bootstrap stack guard probe. Re-flash a production image
after any bringup run — the probe panics by design.

Lab procedure (re-run after layout changes):

```bash
cargo build --release --features bringup
llvm-objcopy -O binary target/aarch64-unknown-none-softfloat/release/harbor-kernel \
  target/aarch64-unknown-none-softfloat/release/kernel8-bringup.img
./scripts/deploy-sd.sh /run/media/$USER/bootfs \
  target/aarch64-unknown-none-softfloat/release/kernel8-bringup.img
```

## RNG200 and SPI0 (hardware)

| Check | Status | Evidence |
| --- | --- | --- |
| RNG200 polled word + soft fail on absence | **closed (HW)** | Pi 4B 2026-08-05 — `rng200: ok word=…`; QEMU — `unavailable (NotPresent)` via `arch::probe` |
| SPI0 pinmux + FIFO self-test + resident handle | **closed (HW)** | Pi 4B `--features debug-display`, 2026-08-05 — bus line before panel bring-up |
| ILI9486 init + fill (regwidth-16 wire) | **closed (HW)** | Pi 4B + Waveshare-class HAT, 2026-08-05 — bare 8-bit cmds → noise/lines; **reg16** framing (`0x00,op`) + RGB565 pixels → clear colour bars; SPI 8 MHz; CS session (ADR-0010) |
| Status surface (banner + slots) | **closed (HW)** | Same session: banner readable; product boot = `HARBOR` fill + status text (colour bars kept as lab API only) |


### Silicon transcript (debug-display, no HAT)

Pi 4B, CP2104 @ 115200, image built with `--features debug-display`, 2026-08-05.
`CNTFRQ=54000000` is silicon. Guard address moves with `.text` and is not tracked
as an invariant.

```
Harbor: hello
EL1 · W^X map · heap · timer + UART RX IRQ · WFI idle
DTB at 0x2eff1f00
MMU on  (W^X, guard page at 0xa4000, 40960 B of table arena left)
DTB mapped: 61440 bytes at 0x2eff1000
heap remaining = 67108864 bytes
rng200: ok word=0xdc62f9e3
SPI0 ready  cdiv=32  bit_clk=15625000 Hz (debug-display)
CNTFRQ=54000000 Hz  timer=10 Hz  PPI=30
IRQs enabled (timer + UART RX)
idle: WFI when no RX/tick work
heap: … fully reclaimed …
unmap: remapped and freed
split: page at 0x200000 split 1, remapped
sched: spawned task-a
sched: spawned task-b
arena: 1 splits, 8 tables free
task-a 0
task-b 0
…
ticks=10
…
```

What those lines claim:

- **`rng200: ok word=`** — presence probe succeeded, warm-up completed, FIFO
  produced a 32-bit sample. Not a CSPRNG claim (see `hardware.md`).
- **`SPI0 ready  cdiv=32  bit_clk=15625000`** — early no-HAT bus self-test
  (500 MHz core / 16 MHz ceiling → CDIV 32). HAT product image logs
  `display: ILI9486 up  cdiv=…  bit_clk=… Hz  status` after panel bring-up
  (lab ceiling 8 MHz until raised with glass re-check).
- **Panel on glass (HAT):** PiScreen-class **regwidth=16 / buswidth=8** is
  required. Logical cmd/param bytes expand to BE `u16` (`0x00,b`); RAMWR
  payload stays raw RGB565. User-confirmed 2026-08-05: distinct colour bars +
  status banner (proof); product path is navy + status text only.
- **M3 / unmap / split** still healthy on the same boot (regression check).

QEMU counterpart (default image, no feature): after MMU,
`rng200: unavailable (NotPresent)` — `arch::probe` recovered the external abort
instead of panicking. That path is documented in the table at the top of this
file; it is not a silicon pass for entropy.

Deploy lab image without the featureless rebuild trap:

```bash
make FEATURES=debug-display img
make FEATURES=debug-display deploy SD_MOUNT=/run/media/$USER/bootfs
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

`CNTFRQ=54000000` says this is silicon, not TCG; the guard at `0xa1000` says
this image is the split layout and not a stale card — the check being "does it
match the image just flashed", never "does it match today's build". Timer IRQs arrive, which is the part worth
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
| Cooperative interleaving (M3)                                    | make `sched::yield_now` a no-op             | `task output not interleaved:` with an empty list — idle spun on `has_ready` and no worker ever ran                                   |
| Block split (M3)                                                 | aim the split smoke at an already-L3 page   | `block split path did not run: split: page at 0xb5000 split 0, remapped` — the line is there, the split is not                        |
| `Context` / assembly coupling (M3)                                | swap `x30` and `sp` in `Context`            | two `offset_of` asserts red at compile time, naming both offsets; the size assert alone stayed green at 104 bytes                     |
| Table-arena reserve (M3)                                         | raise `MIN_SPARE_TABLES` to 40              | `BOOT REFUSED: table arena nearly exhausted: 10 tables left, need 40 (raise PAGE_TABLE_ARENA_SIZE in link.ld)` and then nothing       |
| SPI divisor overflow                                             | range-check after rounding instead of before | `left: Ok(0)` against `right: Err(TargetTooSlow …)` — a wrapped divider is a *legal* encoding, so the fastest request became the slowest clock |
| MMIO probe window (`FAR` match)                                  | drop the `far != expected` check, and fault twice inside one probe | without it both aborts are swallowed and the boot continues (`rng200: unavailable`); with it the second is fatal — `ESR=0x96000050 FAR=0xfe105000`, the injected address |

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

**Dual dongle (PC + Pi USB):** plugging a second USB–serial into a Pi USB port
(or null-modeming two adapters together) does not give Harbor a second console.
The kernel only drives PL011 on GPIO 14/15; bare metal has no USB host/CDC.
Keep the lab path as PC adapter ↔ header UART ([`hardware.md`](hardware.md#serial-console)).
The on-Pi dongle is for Linux-side work only.
