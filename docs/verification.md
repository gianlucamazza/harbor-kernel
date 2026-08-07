# Verification

What is checked, by what, and — the part that matters — what each check cannot
see. A gate whose blind spots are undocumented gets trusted for things it never
covered.

## How to read this file

It is an **index of evidence**, not onboarding, and it is long because
transcripts are kept rather than summarised. Nobody should read it end to end.

| If you want… | Go to |
| --- | --- |
| What each layer of checking covers **and is blind to** | [The layers](#the-layers) — the one section worth reading in full |
| Why `done (QEMU)` is weaker than `done (HW)` | [What emulation cannot catch](#what-emulation-cannot-catch-with-the-example-that-proved-it) |
| The evidence behind one specific claim | Follow the link from the claim; the section headings are dated |
| Where the gates are still blind | [Checks that have been seen to fail](#checks-that-have-been-seen-to-fail), [Four defects no gate caught](#four-defects-no-gate-caught-2026-08-05), [Mutation testing](#mutation-testing-what-the-tests-actually-cover-2026-08-06) |
| What is *done*, rather than how it was shown | [`roadmap.md`](roadmap.md) — status lives there, not here |

Just arriving at the project: the [root README](../README.md) and the
[5-minute path](README.md#the-5-minute-path) come first. This file answers
"why should I believe it", which is the fourth question, not the first.

## The layers

| Layer                                     | Runs                                                  | Covers                                                                                                                                                                  | Blind to                                                                                                                                                                                                                                                                                                                                                         |
| ----------------------------------------- | ----------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Host unit tests (`make test`)             | `cargo test -p kernel-core`                           | Register encodings (UART, SPI, RNG200, …), allocator arithmetic, GIC index maths, region splitting, the SPSC ring                                                       | Anything that touches hardware, and any _use_ of these functions                                                                                                                                                                                                                                                                                                 |
| Miri (`make miri`)                        | Interprets the host tests                             | Aliasing, provenance and data races in the crate's only `unsafe` — the ring's `UnsafeCell` buffer and `Sync` assertion                                                  | The kernel crate's `unsafe`, which touches MMIO and system registers and cannot be interpreted. **Skips the model checks**, which are safe code and would take hours interpreted — the skip is explicit and carries its reason in the run output                                                                                                                 |
| Product image (`make product-builds`)     | Builds without `oracle`, greps the image              | Diagnostic scaffolding reaching the production surface (rule 9), by the strings the demos print — derived from `demos.rs`, so the marker set cannot drift from the code | Scaffolding that leaks without printing anything; the symbol check is a second, weaker net                                                                                                                                                                                                                                                                       |
| EL0 programs (in `make test`)             | Assembles the intended text, compares bytes           | That the bytes an agent runs are the instructions the doc-comment claims — `llvm-mc` assembles, so nobody transcribes hex in either direction                           | Whether the program is the _right_ one for the test; only that it is the one written down                                                                                                                                                                                                                                                                        |
| Bounded model check (in `make test`)      | Replays every operation sequence to a bound           | The scheduler's invariants and the authority core's agreement with a reference implementation, over all sequences rather than chosen ones                               | Anything outside `kernel_core::{tasks, ipc}`, anything past the bound, and any `unsafe` — the walk is over safe code                                                                                                                                                                                                                                             |
| Bring-up build (`make bringup-builds`)    | Compiles and lints `--features bringup`               | A configuration nothing else builds, and the one you reach for when the board will not talk                                                                             | Anything the gates do not _run_ — it compiles, it is not executed                                                                                                                                                                                                                                                                                                |
| No-SIMD guard (`make no-simd`)            | Disassembles the linked image                         | A build that silently regains FP/SIMD                                                                                                                                   | FP that never reaches the image                                                                                                                                                                                                                                                                                                                                  |
| No-`static mut` (`make no-static-mut`)    | Greps `src/**/*.rs` for declarations                  | A `static mut` reintroduced after ADR-0019 landed the last one as an `AtomicPtr`                                                                                        | Prose that names the form; coupling that is not a declaration                                                                                                                                                                                                                                                                                                    |
| IRQ scope (`make irq-scope`)              | Walks each `cpu::without_irqs(` region brace-by-brace | A task switch inside a masked region — the `DAIF` pair would span it and hand the next task this task's mask                                                            | Indirect switches: a call that parks three frames down is invisible to a lexical check                                                                                                                                                                                                                                                                           |
| Pre-MMU path (`make no-early-exclusives`) | Disassembles `_start` and its callees                 | Atomic read-modify-write before translation is on, the path growing, and any indirect branch on it                                                                      | Nothing on that path: an edge it cannot follow is refused rather than skipped                                                                                                                                                                                                                                                                                    |
| QEMU boot (`make boot-check`)             | Boots the image, asserts on the log                   | MMU activation, allocator reclaim, timer IRQ, WFI idle, unhandled interrupts, panics                                                                                    | **Memory attributes.** Also cache behaviour, real clocks, firmware state. RNG200 is not modelled on `raspi4b` — init reports `NotPresent` via `arch::probe`, not a successful FIFO read. **CI note:** Ubuntu apt QEMU (≤8.2) lacks the `raspi4b` machine; GitHub Actions wraps an Arch-packaged QEMU that includes it. Local Arch/QEMU ≥9 already has `raspi4b`. |
| Doc symbols (`make doc-symbols`)          | Module paths in the descriptive docs                  | A sentence that names `a::b::NAME` after `NAME` moved to another module — path-aware, because the symbol usually still exists somewhere                                 | ADRs and reviews, which are dated records; anything named without a module path                                                                                                                                                                                                                                                                                  |
| Doc claims (`make doc-claims`)            | Compares the docs against the source for facts written twice | The `make check` gate list, the host test count, the module lists, the ADR dates, and the **set** of syscalls in `SECURITY.md`'s authority table — a call the kernel decodes and the threat model omits is a call nobody considered | Whether a claim is *true*, only whether the two copies agree. `4 \| SYS_RECV \| (non-blocking)` stayed green the day `SYS_RECV` learned to block: the row was there and the number was right |
| Layering (`make layering`)                | Every `crate::` import edge in `src/`                 | The rules in `architecture.md`: drivers never know the board, arch never names a driver, `exception` reaches only `irq`                                                 | Coupling that is not an import — a shared constant, an agreed register value, a naming convention                                                                                                                                                                                                                                                                |
| Hardware                                  | A Pi 4B on a serial console                           | Everything above, for real                                                                                                                                              | Only what you actually boot and look at                                                                                                                                                                                                                                                                                                                          |

**One assertion in the boot check cannot always be answered, and now says so.**
TCG emulates the guest timer against wall-clock time, so `timer: MISSED` fires
when the machine running QEMU is too busy to execute the guest — observed at
load average 4 during a background `cargo install`, clean on the same image a
minute later.

For a while this was a comment telling the reader to re-run before believing the
red. That is an invitation to ignore a failing gate, and there is not another one
anywhere in this project. The check now corroborates instead of advising: it
measures the host CPU the emulator actually received and reports a **third
outcome** beside pass and fail.

| Outcome       | Meaning                                                       | Exit |
| ------------- | ------------------------------------------------------------- | ---- |
| clean         | every assertion held                                          | 0    |
| FAIL          | a deadline was missed and the emulator had the CPU to meet it | 1    |
| INDETERMINATE | a deadline was missed on a host that starved the emulator     | 3    |

Indeterminate is non-zero on purpose. The run did not establish its claim, and
an unestablished claim must not read as a verified one.

Two candidate signals were tried and discarded before the third worked, which is
worth recording because both look reasonable:

- **Load average** was 4 on this machine while the boot was clean. It measures
  the machine, not this process, and the load sat on other cores.
- **The guest's own tick reports.** TCG drives the guest timer from wall-clock
  time, so the count tracks how long the run lasted rather than how much CPU it
  received: under a 20% cgroup quota the guest still reported 13 ticks while
  running on a fifth of one core.

What separates the cases is the host CPU the emulator was given, read from
`/proc/self/stat` (`cutime` + `cstime`) with no added dependency. Measured: 2.97
cores idle, 0.07 cores under the 8% quota where `timer: MISSED` first appears —
two orders of magnitude, so the one-core threshold sits nowhere near either
edge.

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
`scripts/check/pre-mmu-path.sh` fails the build if anything re-enters it.

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
invalidation is _necessary_ for guards; a deliberate “strip TLBI and re-run”
mutation is still optional if you want a pure TLB-only experiment.

## Protections are only verified when you have seen them fire

W^X and the guard page are claims about what _fails_. A map that reports itself
active proves nothing about enforcement. Both were checked by temporarily
adding a deliberate fault to `bootstrap::run` and booting on hardware:

| Probe                        | ESR          | Decoded                                                        | FAR       | Layout when run                        |
| ---------------------------- | ------------ | -------------------------------------------------------------- | --------- | -------------------------------------- |
| Write to `.text` (`0x80000`) | `0x9600004F` | EC 0x25 data abort, DFSC `0b001111` permission fault L3, WnR=1 | `0x80000` | any — `.text` starts at the image base |
| Write to the guard page      | `0x96000047` | EC 0x25 data abort, DFSC `0b000111` translation fault L3       | `0xa1000` | guard at `0xa1000`, pre-M3             |
| Kernel stack overflow        | `0x96000047` | EC 0x25 data abort, DFSC `0b000111` translation fault L3       | `0xa1ff8` | guard at `0xa1000`, pre-M3             |

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
protect. Re-run the two guard rows when the _mechanism_ changes — a different
guard strategy, a different stack arrangement — not when the address moves.

The probes are not in the tree — a deliberate fault is a dead board. Re-run
them by hand after changing `link.ld` or the region list in `mm::layout`. This
table is the only copy: it used to be duplicated in `mmu.md`, and both copies
went stale together the moment the layout moved.

## M3 cooperative tasks (hardware)

| Check                           | Status          | Evidence                                                             |
| ------------------------------- | --------------- | -------------------------------------------------------------------- |
| Interleaved yield + unmap smoke | **closed (HW)** | Pi 4B serial, 2026-08-04 — transcript below                          |
| Task-stack guard fault          | **closed (HW)** | bringup image, 2026-08-05 — ESR table below                          |
| Review                          | desk done       | [2026-08-04-m3-incremental.md](reviews/2026-08-04-m3-incremental.md) |

QEMU remains gated by `boot-check`. Both silicon rows above are closed: M3 may
be marked `done (HW)`.

## M4 IPC + capabilities

| Check                                    | Status                 | Evidence                                                                                 |
| ---------------------------------------- | ---------------------- | ---------------------------------------------------------------------------------------- |
| ADR-0008 cookie handlers + wake queue    | **closed**             | `Handler = fn(IrqCookie)`; `WakeQueue` host-tested; `poll_wakes` in idle                 |
| Message across tasks (no shared payload) | **closed (QEMU + HW)** | `ipc: sent` / `ipc: got tag=1 a=42` — `make boot-check`; Pi 4B user-confirmed 2026-08-05 |
| Send without hold refused + counted      | **closed (QEMU + HW)** | forger → `ipc: refuse count=N` (N≥1); same boot on Pi 4B                                 |
| Silicon                                  | **closed (HW)**        | Pi 4B, `FEATURES=debug-display` image, 2026-08-05 — boot OK (ipc + status path)          |

M4 is **done (HW)**. QEMU remains gated by `boot-check` (includes the three
`ipc:` lines).

## M5 EL0 / address spaces

| Check                               | Status                 | Evidence                                                                      |
| ----------------------------------- | ---------------------- | ----------------------------------------------------------------------------- |
| Named frame pool (ADR-0012)         | **closed (QEMU + HW)** | boot `frames: N free / N …`; pool region in layout                            |
| `prepare_for_el0` + destroy no leak | **closed (QEMU + HW)** | `aspace: prepare ok` / `create/destroy ok` / no `aspace: LEAK`                |
| EL0 own `TTBR0` + `SVC`             | **closed (QEMU + HW)** | `el0: SVC ok  imm=0`                                                          |
| EL0 store to kernel VA → data abort | **closed (QEMU + HW)** | `el0: FAULT ok  ESR=0x9200004f FAR=0x80000` (permission class)                |
| Silicon                             | **closed (HW)**        | Pi 4B + PL011 CP2104, `FEATURES=debug-display`, 2026-08-05 — transcript below |

Desk prep: [reviews/2026-08-05-m5-prep.md](reviews/2026-08-05-m5-prep.md).
Regime: [ADR-0014](adr/0014-ttbr-split-m5.md) (TTBR0-only v1; kernel maps cloned
into the user root; restore kernel `TTBR0` on lower-EL entry via
`mmu::switch_ttbr0` — sole switch implementation).

M5 is **done (HW)**. QEMU remains gated by `boot-check` (the `aspace:` / `el0:`
lines). Architecture done-when is satisfied on both; the product “scheduled EL0
agent” shell is post-M5 (M5-P1…), not a reopen of this stamp.

### Silicon transcript (M5, closed)

Pi 4B, CP2104 @ 115200, image `FEATURES=debug-display` (HAT + PL011), 2026-08-05.
Same ESR/FAR class as QEMU for the fault probe.

```
Harbor: hello
MMU on  (W^X, guard page at 0xab000, 36864 B of table arena left)
frames: 512 free / 512  base=0x40bc000  (2048 KiB pool)
aspace: prepare ok  held=14 (empty=1)  root=0x40bc000
el0: SVC ok  imm=0
el0: FAULT ok  ESR=0x9200004f FAR=0x80000
aspace: create/destroy ok  pool=512
rng200: ok word=…
display: ILI9486 up  cdiv=64  bit_clk=7812500 Hz  status
ipc: sent tag=1 a=42
ipc: got tag=1 a=42
ipc: refuse count=1
ticks=10
…
```

(`held=` and pool base vary with layout; oracle strings are stable.)

Protocol notes load-bearing for silicon:

- User text: `poke_user` + D-cache clean to PoU / I invalidate.
- Lower-EL paths never install a null `TTBR0`; missing session panics.
- Bootstrap still runs the one-shot SVC/fault probes; **M5-P1** adds a
  scheduled task (`el0-task:` lines).

## M5-P / M6 post

<a id="m5-p--m6-post"></a>
<a id="m5-p--m6-v1-qemu"></a>

### Matrix

| Check                                         | Status                 | Evidence                                                                                                                                   |
| --------------------------------------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Dual AS create/destroy                        | **closed (QEMU + HW)** | `aspace: dual create/destroy ok`                                                                                                           |
| Scheduled EL0 + `svc #0` ping                 | **closed (QEMU + HW)** | `el0-task: svc ping` / `el0-task: ok`                                                                                                      |
| Unknown `SVC` imm refused                     | **closed (QEMU + HW)** | `el0-task: svc refuse imm=0x99`                                                                                                            |
| `kernel_core::syscall::decode` (+ `SYS_PUTC`) | **closed**             | host unit tests (168 total suite)                                                                                                          |
| ADR-0013 accepted                             | **yes**                | agent page-sized PL011 only                                                                                                                |
| PL011 agent map + FR load + kill              | **closed (QEMU + HW)** | `pl011-agent: FR read + svc ok` / `killed ok`                                                                                              |
| Concurrent multi-agent shell                  | **closed (QEMU + HW)** | `agents: concurrent ok` (`src/agent`)                                                                                                      |
| Multi-SVC resume (`enter`/`resume`)           | **closed (QEMU + HW)** | `el0-task: resume pings=2`                                                                                                                 |
| `SYS_PUTC` (imm 2)                            | **closed (QEMU + HW)** | `el0-task: putc bytes=2`                                                                                                                   |
| EL0 IRQ save/resume (re-execute)              | **closed (QEMU + HW)** | `el0-task: irq resume irqs=N` (N≥1)                                                                                                        |
| PL011 RX poll empty path                      | **closed (QEMU + HW)** | `pl011-agent: rx poll empty`                                                                                                               |
| PL011 RX ownership + real bytes               | **closed (QEMU + HW)** | LBE inject; `rx own bytes=2`; `rx own begin/end`                                                                                           |
| Silicon (through multi-SVC / M6 v1 map)       | **closed (HW)**        | Pi 4B transcript below                                                                                                                     |
| Silicon (IRQ / putc / RX own)                 | **closed (HW)**        | Pi 4B 2026-08-06 — [four changes of 2026-08-05](#hardware-evidence-the-four-changes-of-2026-08-05-closed); reconfirmed under M7 2026-08-07 |

**RX ownership:** kernel drain suspended, PL011 RX IRQs masked; agent maps the
UART page and polls `DR`. Real bytes via **PL011 LBE** (kernel TX looped to RX)
— not invented ring writes. `resume_rx` re-arms IMSC. Closed on QEMU and on
silicon (issue #1, 2026-08-06). Roadmap:
[architecture.md §Roadmap](architecture.md#roadmap).

### Expected QEMU boot-check lines (post–issue #1)

In addition to earlier M3–M6 oracles, a clean `boot-check` includes:

```
el0-task: resume pings=2
el0-task: putc bytes=2
el0-task: irq resume irqs=…
el0-task: ok
pl011-agent: FR read + svc ok
pl011-agent: rx own begin
pl011-agent: rx poll empty
pl011-agent: rx own bytes=2
pl011-agent: rx own end
pl011-agent: killed ok  pool=…
agents: concurrent ok  pool=…
```

### Silicon transcript (M5-P / M6 v1 map / concurrent / multi-SVC)

Pi 4B, PL011 via CP2104 @ 115200, image `d674792` + `debug-display`, 2026-08-05.
`CNTFRQ=54000000` is silicon. Closed **through multi-SVC resume**; does **not**
include putc / IRQ resume / RX own (those are QEMU-only until the next HW stamp).

```
Harbor: hello
MMU on  (W^X, guard page at 0xac000, …)
frames: 512 free / 512  base=0x40bd000  (2048 KiB pool)
aspace: prepare ok  held=14 (empty=1)  root=0x40bd000
el0: SVC ok  imm=0
el0: FAULT ok  ESR=0x9200004f FAR=0x80000
aspace: create/destroy ok  pool=512
aspace: dual create/destroy ok  pool=512
rng200: ok word=…
display: ILI9486 up  cdiv=64  bit_clk=7812500 Hz  status
…
sched: spawned el0-task
sched: spawned pl011-agent
sched: spawned agent-a
sched: spawned agent-b
…
el0-task: svc ping
el0-task: svc refuse imm=0x99
el0-task: resume pings=2
el0-task: ok
pl011-agent: FR read + svc ok
pl011-agent: killed ok  pool=512
agent-b: svc ping
agent-a: svc ping
agents: concurrent ok  pool=512
ipc: sent tag=1 a=42
ipc: got tag=1 a=42
ipc: refuse count=1
ticks=10
…
```

Multi-SVC also closed on silicon with image `223e34f`.

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

| Field | Value            | Meaning                                                      |
| ----- | ---------------- | ------------------------------------------------------------ |
| ESR   | `0x96000047`     | EC 0x25 data abort; DFSC `0b000111` **translation fault L3** |
| FAR   | `0xc2ff8`        | top of **self** guard `[0xc2000, 0xc3000)`                   |
| Peers | `0xb7…`, `0xbd…` | FAR is **outside** both peer stacks                          |

Same DFSC class as the bootstrap stack guard probe. Re-flash a production image
after any bringup run — the probe panics by design.

Lab procedure (re-run after layout changes):

```bash
cargo build --release --features bringup
llvm-objcopy -O binary target/aarch64-unknown-none-softfloat/release/harbor-kernel \
  target/aarch64-unknown-none-softfloat/release/kernel8-bringup.img
./scripts/host/deploy-sd.sh /run/media/$USER/bootfs \
  target/aarch64-unknown-none-softfloat/release/kernel8-bringup.img
```

## RNG200 and SPI0 (hardware)

| Check                                          | Status          | Evidence                                                                                                                                                                     |
| ---------------------------------------------- | --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| RNG200 polled word + soft fail on absence      | **closed (HW)** | Pi 4B 2026-08-05 — `rng200: ok word=…`; QEMU — `unavailable (NotPresent)` via `arch::probe`                                                                                  |
| SPI0 pinmux + FIFO self-test + resident handle | **closed (HW)** | Pi 4B `--features debug-display`, 2026-08-05 — bus line before panel bring-up                                                                                                |
| ILI9486 init + fill (regwidth-16 wire)         | **closed (HW)** | Pi 4B + Waveshare-class HAT, 2026-08-05 — bare 8-bit cmds → noise/lines; **reg16** framing (`0x00,op`) + RGB565 pixels → clear colour bars; SPI 8 MHz; CS session (ADR-0010) |
| Status surface (banner + slots)                | **closed (HW)** | Same session: banner readable; product boot = `HARBOR` fill + status text (colour bars kept as lab API only)                                                                 |

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
property that matters: an unmapped page catches an overflowing _read_ too.

It took two runs. The first was captured while a stale monitor still held the
port, and the two readers split the stream — `CNTFRQ=5400096000047` is one line
of each. The bytes could have been stitched back together from the two logs, and
the answer would have been right, but a reconstructed stream is what produced a
wrong conclusion earlier in this project. The probe was re-run with one reader
instead.

The W^X probe needs no re-run: `.text` and `.rodata` were not touched by the
split, and its recorded ESR does not depend on an address that moved.

## Hardware evidence: the loader and the park, on silicon (2026-08-07)

Pi 4B, 2026-08-07 12:10, `.serial-log/20260807-120838.log`, image
`b5c78784…1067` (91360 B), commit `741137e`. One boot carrying both ADR-0021 and
ADR-0022.

`CNTFRQ=54000000 Hz` says silicon rather than TCG — QEMU reports 62500000 for
the same board — and `rng200: ok word=0x5bb0c241` says the same thing a second
way: the emulator has no backend and reports `unavailable (NotPresent)`.
`reset: PowerOn partition=0` says a cold start rather than a watchdog covering
for something.

### ADR-0021: authority is one entry in a table, on real hardware

```
12:10:11.070397 loader: echo loaded text=1 stack=3
12:10:11.070607 loader: mute loaded text=2 stack=3
12:10:11.092824 H!loader: echo ran putcs=2 refusals=0
12:10:11.092921 loader: mute ran putcs=0 refusals=2
```

Two tasks, **one image** — the same `const [u8; 32]` in `.rodata` — and the only
difference between them is whether the manifest put the loader's console
capability in slot 1. `echo` printed `H!` through the capability it was bound;
`mute` was refused twice.

`mute` ran with **two** text pages. That is the part silicon had to answer:
`AddressSpace::poke_user` writes a multi-page image page by page and publishes
each range for instruction fetch (D clean to PoU + I invalidate), and an
emulator would have run the program whether or not those maintenance operations
were there — QEMU's caches are coherent by construction and a Cortex-A72's are
not. `mute` reached its `SYS_PUTC` calls and was refused by the capability check
rather than faulting on a stale instruction fetch, so the second text page was
really mapped `USER_RX` and really published.

### ADR-0022: the agent waited, and the send woke it

```
54  12:10:11.115220 el0-ipc: try-recv empty without waiting empties=1
55  12:10:11.115347 el0-ipc: sent slot=0 tag=7 a=42
65  12:10:11.156757 *el0-ipc: got payload via EL0 recvs=1
```

Line numbers 54 < 55 < 65, which is the assertion `boot-check` makes and the
reason it stopped checking presence alone. The receiving agent is spawned
**first** and opens with no `yield_now`: line 54 is `SYS_TRY_RECV` on its own
slot reporting the mailbox empty, so the wait that follows is a wait. The
payload arrives 41 ms later, after the peer posted.

The `DAIF` scoping change is what silicon tests here that QEMU cannot argue
about. The session loop now takes and releases the interrupt mask once per
enter/resume step, on a core with a real exception entry and a real
`msr daifset`/`msr daifclr` pair — and the timer kept ticking to 130 with no
storm and no stall.

### Absences, counted

| Absence                          | Count | What its presence would have meant                                                             |
| -------------------------------- | ----- | ------------------------------------------------------------------------------------------------ |
| `panic`                          | 0     | Any assertion fired, including `el0: published session is not the current task's`               |
| `no published session`           | 0     | The vector path read a stale pointer across the park's switch — the one ADR-0019 guards         |
| `Xel0`                           | 0     | The console-less agent's byte reached the UART                                                   |
| `loader: … FAILED` / `refused`   | 0     | An entry the manifest declared could not be bound or created                                     |

`ipc: refuse count=5 full=0 state=0`. Five is exact, not a floor: the M4 forger,
the EL0 agent's unheld slot, its denied console, and `mute` twice. `state=0` is
the idle-park guard reporting it was never needed, which is the only honest
thing it can report.

### Costs nothing

`pool=496` at the concurrent peak and `pool=512` after the kill — identical to
every session since 2026-08-06, across two more tasks, a two-page text window
and a parked agent. `arena: 1 splits, 23 tables free` against a reserve that
grew with `MAX_TASKS`. The PL011 handover still completes:
`rx own bytes=2`, `killed ok pool=512`.

### Honest limit

The park is exercised between two tasks that never hold live EL0 sessions
simultaneously — each parks with its session saved and nothing enters EL0 in
between. Per-task session state makes such an overlap harmless and nothing
performs one. A preemptive scheduler is what would, and preemption remains
**open completeness track K4** ([ADR-0026](adr/0026-kernel-and-product-completeness.md))
— cooperative until a successor to ADR-0006.

## The manifest: same bytes, different authority (2026-08-07, QEMU)

ADR-0021 landed. The claim is that authority lives in a table rather than in a
program or in the code that spawns it, and the smallest form of that claim is two
entries running the **identical image**:

```
loader: echo loaded text=1 stack=3
loader: mute loaded text=2 stack=3
H!loader: echo ran putcs=2 refusals=0
loader: mute ran putcs=0 refusals=2
```

`echo` and `mute` share one `const [u8; 32]` in `.rodata` — the same bytes,
built by `prog::encode_putc_hi_exit`, which the assembler oracle already checks
against `llvm-mc`. `echo` prints `H!`. `mute` is refused twice. The only
difference between them is whether the manifest put the loader's console
capability in slot 1.

`mute` also declares **two** text pages against `echo`'s one, so the boot
exercises a window geometry the BSP no longer fixes — and a multi-page text is
exactly why `AddressSpace::poke_user` now walks pages instead of writing from
page 0's physical address. The frames behind a window are adjacent only by
accident of the pool's LIFO free order: that accident holds on a fresh boot and
stops holding after the first create/destroy cycle, which is the shape of bug
this change could have re-introduced and does not.

### Seen red

| Change                                             | What failed                                                                                                                                                         |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `echo`'s slot 1 set to index `9`, loader holds one | `loader: echo refused — slot 1 names capability 9 of 1`, and the boot continued. Not a panic, not a silent `None`, and not a read past the end of the loader's list |
| The loader landed with `MAX_TASKS` still 12        | `loader: echo spawn FAILED Full` — the oracle was already at exactly twelve tasks                                                                                   |

The first is the assertion the manifest exists for. It is arithmetic:
`index >= held.len()`. An entry cannot name authority the loader does not hold,
and that is a property of the shape rather than of a check somebody has to
remember to write.

### Numbers

`ipc: refuse count=5`, up from three. The two new ones are `mute`'s, and the
count is asserted exactly rather than as a range — a range would let any one
producer satisfy the assertion for the others, which it once did.

`make product-builds` fell from **95 unreachable items to 37** (36 once the
manifest index left the TCB): the loader is
product code and calls `spawn_with_slots`, `AddressSpace`, `Agent` and the EL0
session. The **image size did not move** — 54496 B before and after — because
the manifest is `cfg(oracle)` and an empty table loads nothing. Reachable in the
source, absent from the image. That is the honest state of ADR-0021's positive
claim until M8 gives the product an agent.

## Blocking `SYS_RECV`: what the oracle stopped arranging (2026-08-07, QEMU)

ADR-0022 landed. The property is not "a message crossed" — that was already
true. It is **that the receiving agent got there first and waited**, which the
oracle previously arranged not to test:

```rust
// before
crate::sched::yield_now();   // let the sender post first
crate::sched::yield_now();
```

Those two lines, plus a spawn order that put the sender first, made the exchange
work whether or not a blocking recv did. Both are gone. The receiver is spawned
first, opens with nothing, and the boot check asserts three lines **in order**:

```
49  el0-ipc: try-recv empty without waiting empties=1
50  el0-ipc: sent slot=0 tag=7 a=42
60  *el0-ipc: got payload via EL0 recvs=1
```

Line 49 is `SYS_TRY_RECV` on the same slot, and it is what makes the rest an
argument rather than a coincidence: the mailbox really was empty when the agent
arrived. Line 50 is the peer posting eleven lines later. Line 60 is the parked
agent resuming with the payload.

`grep -qa` could not have said this. The script compared presence only, and
presence is satisfied by any interleaving — so the ordering is now compared by
line number, and the failure message prints the three numbers it found.

### Seen red, twice

| Change                                                        | What failed                                                                                                             |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `SYS_RECV` back to `try_recv_from_slot`, receiver still first | `boot-check: FAIL — EL0 agent did not receive the message through its slot`. Without the park the payload never crosses |
| `sched::yield_now()` inside `resume_step`'s `without_irqs`    | `irq-scope: src/agent/mod.rs:178: `yield_now`is inside the`without_irqs` opened at line 177`, exit 1                    |

The second is the one worth insisting on. A scope check is easy to write in a
form that matches nothing, and this one had to be seen naming a file, the line
of the offending call, **and** the line of the region that contains it — a
region opened one line earlier here, but forty lines earlier in the shape the
gate exists to catch.

### What the gate cannot see, and is not claimed to

`irq-scope` is lexical. `ipc::recv_from_slot` switches — it parks — but three
frames down, so a call to it inside a masked region passes. Catching that needs
a call graph this tree does not have. What is bought is that the _direct_ form,
which is how the mistake is actually written, cannot land unnoticed; the
indirect form is review's, and it is listed in the gate blind spots above rather
than left to be found.

### Numbers that did not move, and one that did

`refuse count=3 full=0 state=0` — unchanged. The park added a fourth way to be
refused (`Status::Busy`, two waiters on one endpoint) and a fifth counter path
(the idle guard, a state refusal), and neither fires: nothing creates a second
waiter, and idle does not run agents. `state=0` is that guard reporting it was
never needed, which is the only honest thing it can report.

`make product-builds` moved from 88 unreachable items to **95**. The park is
product code the product cannot reach either — `recv_from_slot`, the `TryRecv`
arm, the wake path — because nothing in the product creates an agent. The number
going _up_ is the loader argument getting stronger, not weaker.

## Hardware evidence: `main` after ADR-0019 — the atomic on the vector path (2026-08-07)

Pi 4B, 2026-08-07 10:24, `.serial-log/20260807-102411.log`, image
`e96a4fb8…3e21` (83168 B), commit `09289c5`.

This boot exists for one reason: **`main` had never run on silicon.** The last
transcript was `f951f6a`, eleven commits earlier, and in between ADR-0019 turned
`CURRENT_EL0` from a `static mut` into an `AtomicPtr`. That symbol is not
ordinary state — `vectors.s` dereferences it on **every** exception taken from
EL0:

```asm
adrp x16, CURRENT_EL0
add  x16, x16, :lo12:CURRENT_EL0
ldr  x16, [x16]
```

The Rust side now stores with `Release` and loads with `Acquire`; the assembly
side does a plain `ldr` and always did. The question hardware answers and an
emulator cannot is whether that plain load sees the pointer the scheduler
published, with real caches, a real exception entry, and no TCG serialising
everything into one order. QEMU would agree with either a correct atomic or a
broken one.

**It sees it.** Every oracle line of the M7 stamp reproduced, at the same
counts:

```
10:24:17.355038 el0-task: resume pings=2
10:24:17.355171 H!el0-task: putc bytes=2
10:24:17.355293 el0-task: irq resume irqs=1
10:24:17.376611 el0-ipc: console denied, printed nothing
10:24:17.376866 el0-ipc: agent faulted esr=0x9200004f far=0x80000 faults=1
10:24:17.376999 el0-ipc: creator alive after fault
10:24:17.398976 RXagents: concurrent ok  pool=496
10:24:17.399245 ipc: refuse count=3 full=0 state=0
10:24:17.399382 *el0-ipc: got payload via EL0 recvs=1
10:24:17.401637 pl011-agent: killed ok  pool=512
```

`CNTFRQ=54000000 Hz` says silicon rather than TCG; `reset: PowerOn
partition=0 (PM_RSTS=0x00001000)` says a cold start rather than a watchdog
recovering from something. 206 tick reports to 10:27:43, no storm, no stall.

### Three absences, asserted rather than assumed

| Absence                          | Counted | What its presence would have meant                                                                                             |
| -------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `panic`                          | 0       | Any assertion fired — including `el0: published session is not the current task's`, the tripwire that guards the stale pointer |
| `no published session`           | 0       | The vector path read a null through the new atomic: the exact failure ADR-0019 could have introduced                           |
| The denied byte before `el0-ipc` | 0       | The console-less agent's write reached the UART. The five `X` in this log are all `W^X` and `RX` banners, matched as `Xel0`    |

The third row is stated as a pattern rather than as "no `X` anywhere", which is
how the M7 section put it: that log contains `RX` banners too, so the loose
phrasing happened to be checked correctly and could have been checked wrongly.

### What is new in this image and visible in the transcript

```
10:24:17.175822 build: headless (no SPI TFT, no bring-up gates)
```

The banner is the product of the `oracle`/`bringup`/`debug-display` split
(rule 9): an image now says what it is on the wire, before a blank panel or a
missing probe has to be diagnosed from absence. This is the first hardware boot
where the kernel declares its own build.

### What this does not establish

No agent enters EL0 while another's session is live — the loop still runs inside
`cpu::without_irqs`, so the atomic is exercised across **task switches** but not
across a preemption inside a session. The `Release`/`Acquire` pair is therefore
verified where the code uses it today and not beyond that. A blocking
`SYS_RECV` is the change that would exercise the rest.

## Hardware evidence: M7 closed on silicon (2026-08-07)

Pi 4B, 2026-08-07 00:05, `.serial-log/20260807-000115.log`. One boot carrying
all four slices. The milestone's done-when reads _two EL0 agents exchange a
message neither can forge; one of them faults; its creator handles the fault and
the other keeps running; the kernel stays alive_, and this is that sentence:

```
00:05:10.342620 console: capability minted
00:05:10.387884 H!el0-task: putc bytes=2
00:05:10.388004 el0-task: irq resume irqs=1
00:05:10.409069 el0-ipc: sent slot=0 tag=7 a=42
00:05:10.409324 el0-ipc: console denied, printed nothing
00:05:10.409471 el0-ipc: refused slot=1 authority=2
00:05:10.409613 el0-ipc: agent faulted esr=0x9200004f far=0x80000 faults=1
00:05:10.409735 el0-ipc: creator alive after fault
00:05:10.431308 RXagents: concurrent ok  pool=496
00:05:10.431389 *el0-ipc: got payload via EL0 recvs=1
00:05:10.431459  tpl011-agent: rx own bytes=2
00:05:10.434355 pl011-agent: killed ok  pool=512
```

### What the timestamps prove that the lines alone do not

The peer's `got payload` is at **00:05:10.431389**, the fault at
**00:05:10.409613**. Twenty-two milliseconds and eleven lines apart, in that
order. The claim is not "both happened" — it is that the fault did not stop the
other agent, and only the ordering says so.

Likewise `console denied, printed nothing` at .409324 and `sent slot=0` at
.409069: the same agent that successfully used the capability it holds was
refused the one it does not, in the same session, milliseconds apart. The
refusal is not a program that fails at everything.

### Three absences, each asserted

| Absence                    | What its presence would have meant                                                                                                    |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| No `X` anywhere in the log | The denied agent's byte reached the UART — a capability check that returns a status and performs the action anyway                    |
| No panic                   | The `CURRENT_EL0` assertion never fired across five agents in four tasks, so the scheduler published on every switch that reached EL0 |
| `full=0 state=0`           | No mailbox filled and no endpoint resolved to a dead slot; the three authority refusals are the three the boot performs deliberately  |

The counters read `authority=2` at the EL0 agent's own refusal and `count=3` at
the end: the console denial, the unheld slot, and the M4 forger. Exactly three,
asserted as three — the number that a bug in the counter made unreliable a day
earlier, and that `Table::note_authority_refusal` fixed.

### Costs nothing

`pool=496` at the concurrent peak and `pool=512` after the kill, matching the
2026-08-06 sessions at 15:43 and 21:25 exactly. Four slices — per-task session
state, the slot ABI, the console capability, the fault policy — and the frame
pool does not move. `arena: 1 splits, 23 tables free` against a reserve of 14 is
likewise unchanged.

### Honest limit

The two EL0 agents _interleave_ with each other and with `task-a`/`task-b`, but
neither enters EL0 while the other's session is live: the loop still runs inside
`cpu::without_irqs`. Per-task session state makes such a switch harmless and
nothing performs it. A blocking `SYS_RECV` is what would, and it is deliberately
not in M7 — see ADR-0017's consequences.

## Hardware evidence: M7 slice 1, per-task EL0 sessions (closed)

Pi 4B, 2026-08-06 21:25, `.serial-log/20260806-212223.log`. The first boot after
the nine machine-wide `static mut` became one `El0Session` per task behind one
published pointer (ADR-0017 §1).

```
21:25:12.061376 Harbor: hello
21:25:12.061960 reset: PowerOn partition=0 (PM_RSTS=0x00001000)
21:25:12.084894 el0: SVC ok  imm=0
21:25:12.085029 el0: FAULT ok  ESR=0x9200004f FAR=0x80000
21:25:12.215606 el0-task: svc ping
21:25:12.215703 el0-task: svc refuse imm=0x99
21:25:12.236377 el0-task: resume pings=2
21:25:12.236607 H!el0-task: putc bytes=2
21:25:12.236739 el0-task: irq resume irqs=1
21:25:12.237034 pl011-agent: rx own begin
21:25:12.237121 agent-b: svc ping
21:25:12.237485 agent-a: svc ping
21:25:12.256232 RXagents: concurrent ok  pool=496
21:25:12.256332 ipc: got tag=1 a=42
21:25:12.256744  tpl011-agent: rx own bytes=2
21:25:12.256938 pl011-agent: killed ok  pool=512
```

Every EL0 oracle the previous hardware session produced, produced again from
per-task session state, byte for byte where it is a count: `resume pings=2`,
`putc bytes=2`, `irq resume irqs=1`, `rx own bytes=2`, `concurrent ok pool=496`,
`killed ok pool=512`. The two pool numbers match the 2026-08-06 15:43 session
exactly, so the change costs no frames.

**What the absence proves.** `arch::el0` panics if the published session is not
the one the caller named, and every EL0 entry on this boot went through that
check — five agents (`el0-task`, `pl011-agent`, `agent-a`, `agent-b`, and
bootstrap on idle) across four separate tasks. No panic, so the scheduler
published correctly on every switch that reached EL0. Deleting that publication
panics on the first spawned-task entry (see the checks-seen-to-fail table), so
the silence is a result rather than an untested path.

**What it does not prove.** Two agents _interleave_ here — `agent-b: svc ping`,
`agent-a: svc ping` and `task-a`/`task-b` between them — but each still enters
EL0 inside `cpu::without_irqs`, so no switch happens while a session is _live_.
Per-task state makes that switch harmless; nothing yet performs it. That is
M7 slice 2's evidence to produce, not this one's.

**Note on the capture, not the kernel.** The first boot of this session
(21:22:04) is in `.serial-log/20260806-212133.log` and stops after 36 lines, mid
bring-up: the capture had been started through `| head -40`, which closed the
pipe and killed the recorder while the board kept running. Nothing was wrong
with the boot; the transcript simply did not exist for it, which is the same as
not having run it. Re-run from a power cycle with the recorder unpiped.

## Hardware evidence: the four changes of 2026-08-05 (closed)

Four changes from the multi-role review had never run on silicon, and QEMU is
documentedly blind to the class each of them touches. Closed on a Pi 4B,
2026-08-06, over five boots: two bring-up and three production.

**`SCTLR_EL1` RES1 bits.** The image writes `0x30d01805`, where the previous
value was `0x1005`. QEMU does not force the ARMv8.0-A RES1 bits, and an A72
would be within its rights to. The bring-up gate reads the register back:

```
selftest: SCTLR_EL1=0x30d01805 RES1=0x30d00800/0x30d00800
```

Written is read: the hardware forced nothing beyond the pattern the image
already sets. That is the whole claim — not that the value is _correct_, which
the architecture manual settles, but that no bit arrives from somewhere else.

**Table arena at 32 pages, reserve derived from `MAX_TASKS`.** The number QEMU
reports is against a bring-up `.text`; production is what ships:

```
MMU on  (W^X, guard page at 0xbb000, 102400 B of table arena left)
arena: 1 splits, 23 tables free
```

Twenty-three free against a derived reserve of fourteen. The arena was
previously sized against a reserve of six that assumed `MAX_TASKS = 4`, long
after the scheduler raised it to twelve — see the reversal row for that check
above, which is what caught it.

**GIC programming order (`disable` first).** `config.txt` sets `enable_gic=1`,
so the firmware has already programmed the distributor before any of this
kernel's code runs, and that pre-programmed state is exactly what QEMU does not
reproduce. On the bring-up image:

```
gate: HPPIR=30 ok
inject: IAR=0x1e id=30
inject: ticks 0 -> 2
IRQs enabled (timer + UART RX)
```

**PL011 RX handover.** The hardest of the four, because the window is a couple
of instructions wide and needs a byte to arrive _inside_ it — and the QEMU boot
check types nothing at all. Driven here by streaming a byte every 2 ms into the
board's RX for seven seconds, straddling the whole handover, rather than by
typing: a hand cannot hit a window it cannot see.

The first attempt covered only half of it. The injector was triggered on the
`pl011-agent: rx own begin` line, which the kernel prints _after_ `suspend_rx`
has already returned, so bytes were only ever in flight across `resume_rx`.
Re-run from the boot banner instead, and the suspend side reports itself:

```
pl011-agent: rx own begin
ypl011-agent: rx poll unexpected putcs=1
 tpl011-agent: rx own bytes=2
pl011-agent: rx own end
yyyy…pl011-agent: killed ok  pool=512
yyyy…ticks=10 … ticks=270
```

Three separate things in that excerpt. `rx poll unexpected putcs=1` is an
injected byte reaching the **agent** while the kernel's drain was suspended —
the operational definition of the agent owning RX, and the evidence that bytes
were arriving during the suspended region and not merely after it. `rx own
bytes=2` is the loopback pair still arriving intact underneath the injected
traffic. And the `y`s resuming after `rx own end` are the kernel drain echoing
again.

What must not happen is a storm. With the pre-fix inversion — the IRQ view
disarmed before `IMSC` is masked — a byte in that window re-enters the handler
with the base still zero, so it returns without popping `DR` or writing `ICR`,
and on a level-triggered line the interrupt is never cleared. The tick counter
would stop at the handover. It runs to 270 and beyond.

**Honest limit.** The window is one instruction pair wide and both halves run
inside `cpu::without_irqs`, so whether a byte landed in that exact pair is not
knowable from outside. What is established is that ~3500 bytes crossed the
region, the drain changed hands twice, a byte demonstrably arrived while it was
suspended, and no storm occurred. That is strictly more than the boot check can
say — it types nothing — and strictly less than proof.

**Unexplained, and now answered by the next boot rather than by a guess.**
After the bring-up image's guard probe panicked and halted, the board booted
again on its own. Nothing in this kernel resets it, `*** halt ***` is a `wfe`
loop with IRQs masked, and no power cycle was performed between the two runs.
It did not affect the evidence — the two bring-up boots agree line for line
except the RNG word, which must differ — but a board that restarts after halt
is doing something nobody has accounted for.

Three stories fit it (a firmware watchdog never disarmed, a brownout, a glitch
on the supply) and nothing distinguished them, so the kernel now reads the
register that can. `PM_RSTS` latches the cause of the last reset, and every
boot prints it:

```
reset: PowerOn partition=0 (PM_RSTS=0x00001000)
```

QEMU models the block and reports a power-on. That is worth stating because the
first version of this code assumed the opposite — by analogy with RNG200, which
QEMU does not model — and the first boot refuted it. `ResetCause::None` is a
distinct outcome from `PowerOn` precisely so a register that latched nothing
cannot be reported as a clean power cycle.

The decode is `kernel_core::reset`, with six host tests. The one that carries
the question: a watchdog reset that _also_ sets the power-on bit must read as a
watchdog, because answering `PowerOn` there would get it wrong in the only
direction that costs anything.

Still open, and now cheap to close: reproduce the halt on hardware and read the
line on the boot that follows. `make serial-capture` timestamps every line, so
the interval is recorded too — the picocom transcript could not say how long
after the halt the reboot happened, which is why the question stayed open at
all.

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

| Check                                                            | Mutation                                                                                                                               | Observed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| PL011 divisors, bump alignment, `TCR.EPD1`, descriptor alignment | original implementations                                                                                                               | 10 red tests before the fixes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| SPSC ring ordering                                               | publish `head` before writing the slot                                                                                                 | `out of sequence at 8572`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Allocator coalescing                                             | drop the backward merge                                                                                                                | `arena must be whole again`, `churn left the arena fragmented`                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| L3 descriptor encoding                                           | encode an L3 leaf as a block                                                                                                           | `L3 leaf must be 0b11`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| No-SIMD guard                                                    | the pre-softfloat image                                                                                                                | `dup v0.4h` in `memset`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Pre-MMU path                                                     | a Rust `fetch_add` called from `_start`                                                                                                | named the symbol and explained the fix                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| QEMU boot check                                                  | remove `irq::enable(TIMER_IRQ)`                                                                                                        | missing tick reports                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Trap frame coupling                                              | grow `TrapFrame` by 16 bytes                                                                                                           | the stub's reservation moved `0x110` → `0x120`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Blob integrity                                                   | corrupt an expected hash                                                                                                               | refused to install, exit 1                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Miri                                                             | publish `head` before writing the slot                                                                                                 | `Undefined Behavior: Data race detected between (1) non-atomic write and (2) non-atomic read`                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `mmu::map` overwrite refusal                                     | map the same region twice                                                                                                              | `AlreadyMapped(0x8000000)` instead of a silent replacement                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Bring-up build gate                                              | rename a function used only there                                                                                                      | `make bringup-builds` red, `E0425`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Layout validator                                                 | `GUARD_PAGE_SIZE = 0` in `link.ld`                                                                                                     | `LAYOUT INVALID: GuardIneffective` — and the first attempt at that check passed, which is how the linker-symbol fold below was found                                                                                                                                                                                                                                                                                                                                                                                                    |
| Refusal to boot unprotected                                      | make `mmu::activate` return `OutOfTables`                                                                                              | `BOOT REFUSED: could not map planted failure` and then nothing — no heap line, no ticks, no console loop                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Pre-MMU path, indirect branch                                    | reach the gate through `blr x9`                                                                                                        | `indirect branch in _start: its target is not derivable` — the call graph the check walks had a hole                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Layering rules                                                   | `drivers` imports `bsp`; `arch` imports `drivers`; `exception` imports `drivers`                                                       | one line naming the module and the edge, for each of the three rules separately                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| RX bytes dropped                                                 | shrink the ring to 4 bytes and paste 60                                                                                                | `console: DROPPED 57 received bytes (ring full)`, where before the loss was invisible                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Exception stack (`SP_EL1`)                                       | run the same overflow on the pre-split tree                                                                                            | `FAR=0x9c000`, the guard's **bottom**, against `0xa1ff8`, its **top** — the handler had walked the whole page and landed below it                                                                                                                                                                                                                                                                                                                                                                                                       |
| Exception-stack guard page                                       | zero-length exception guard in `Boundaries`                                                                                            | `GuardIneffective` — validation is written once over both stacks, and this is what keeps that true                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Double-free refusal (the mark)                                   | stop consulting the allocated bit                                                                                                      | one test red — the one where alignment leaves the back-pointer intact, which is the only case the sentinel cannot catch                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Double free through the real allocator                           | free the same pointer twice in `console_loop`                                                                                          | `heap: REFUSED 1 invalid frees`, boot check red, and the heap still `fully reclaimed`                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Doc claims (test count)                                          | restore the stale `54 host unit tests`                                                                                                 | `README claims 54 host unit tests, there are 77` — the exact drift it was written for                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Doc claims (gate list)                                           | drop `bringup-builds` from the README                                                                                                  | printed both lists side by side; this is F27, which had already happened twice for real                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| TLBI operand shift                                               | drop the `>> 12`                                                                                                                       | three tests red — the operand became the address, invalidating a different page                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Runtime mapping (`mmu::map`)                                     | skip the call, keep the read                                                                                                           | `ESR=0x96000006` level-2 translation fault at the blob address; with the call, `0xd00dfeed`                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Cooperative interleaving (M3)                                    | make `sched::yield_now` a no-op                                                                                                        | `task output not interleaved:` with an empty list — idle spun on `has_ready` and no worker ever ran                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Block split (M3)                                                 | aim the split smoke at an already-L3 page                                                                                              | `block split path did not run: split: page at 0xb5000 split 0, remapped` — the line is there, the split is not                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `Context` / assembly coupling (M3)                               | swap `x30` and `sp` in `Context`                                                                                                       | two `offset_of` asserts red at compile time, naming both offsets; the size assert alone stayed green at 104 bytes                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Table-arena reserve (M3)                                         | raise `MIN_SPARE_TABLES` to 40                                                                                                         | `BOOT REFUSED: table arena nearly exhausted: 10 tables left, need 40 (raise PAGE_TABLE_ARENA_SIZE in link.ld)` and then nothing                                                                                                                                                                                                                                                                                                                                                                                                         |
| SPI divisor overflow                                             | range-check after rounding instead of before                                                                                           | `left: Ok(0)` against `right: Err(TargetTooSlow …)` — a wrapped divider is a _legal_ encoding, so the fastest request became the slowest clock                                                                                                                                                                                                                                                                                                                                                                                          |
| MMIO probe window (`FAR` match)                                  | drop the `far != expected` check, and fault twice inside one probe                                                                     | without it both aborts are swallowed and the boot continues (`rng200: unavailable`); with it the second is fatal — `ESR=0x96000050 FAR=0xfe105000`, the injected address                                                                                                                                                                                                                                                                                                                                                                |
| Table-arena reserve, derived                                     | restore `PAGE_TABLE_ARENA_SIZE = 16 * 0x1000` under the reserve now derived from `MAX_TASKS`                                           | `BOOT REFUSED: table arena nearly exhausted: 9 tables left, need 14` — the arena had been sized against a reserve of six that assumed `MAX_TASKS = 4`, long after the scheduler raised it to 12                                                                                                                                                                                                                                                                                                                                         |
| Facade isolation (ADR-0015)                                      | `use crate::arch::riscv64::cpu`, `use crate::{arch::aarch64::cpu, bsp}`, `use crate::bsp::rpi4::memmap` in one file outside both trees | three violations named with their line numbers — the first two were invisible to the gate as first written, which listed `aarch64` literally and looked for the `crate::` prefix a grouped import does not carry                                                                                                                                                                                                                                                                                                                        |
| Arch contract vs facade                                          | delete the `probe` row from `arch-contract.md`                                                                                         | `missing from the contract: probe` — the surface a port is written against and the surface the facade actually re-exports had nothing comparing them                                                                                                                                                                                                                                                                                                                                                                                    |
| IRQ dispatch table seal                                          | register a handler _after_ `irq::seal()`, on the real kernel path                                                                      | `MUTATION: post-seal register -> Err(Sealed { irq: 7 })`, and `irq: sealed with 2 handlers registered` unchanged. The seal is what makes the IRQ path's shared `&'static` borrow sound, and until `kernel_core::irqtable` existed nothing had ever registered after sealing to watch it refuse — the invariant the safety argument rests on was asserted by a comment                                                                                                                                                                   |
| Dispatch table populated                                         | drop the `println!` reporting the seal count                                                                                           | `boot-check: FAIL — dispatch table sealed with the wrong number of handlers: (no seal line at all)`. A boot that registers nothing is indistinguishable from a healthy one until the first interrupt nobody answers                                                                                                                                                                                                                                                                                                                     |
| ADR table in `architecture.md`                                   | run the new `xrefs` check against the table as it stood                                                                                | `0015-multi-arch-scaffold.md is missing from the artefact table`, and the same for 0016. Both had been written, accepted and merged while the table a reader meets first still stopped at ADR-0014 — the third copy of a fact the gate was already comparing in two places                                                                                                                                                                                                                                                              |
| `CURRENT_EL0` published on switch                                | delete `publish_el0(sched, to)` from `switch_with`, boot                                                                               | `panicked at src/arch/aarch64/el0.rs: el0: published session is not the current task's (stale after switch)` on the first EL0 entry from a spawned task. Before slice 1 this row read "Nothing yet" in ADR-0017 — a stale pointer is silent until one agent reads another's saved registers, so the check shipped in the same commit as the pointer                                                                                                                                                                                     |
| No-`static mut` (ADR-0019)                                       | restore `static mut CURRENT_EL0: *mut El0Session`                                                                                      | `no-static-mut: src/arch/aarch64/el0.rs:…: static mut CURRENT_EL0:…` then exit 1 — the gate greps declarations, not prose, so comments that name the form stay green                                                                                                                                                                                                                                                                                                                                                                    |
| IRQ scope (ADR-0022)                                             | put `sched::yield_now()` inside `resume_step`'s `without_irqs`                                                                         | `irq-scope: src/agent/mod.rs:178: \`yield_now\` is inside the \`without_irqs\` opened at line 177` then exit 1 — the region is found by brace depth, so the call does not have to be on the opener's line                                                                                                                                                                                                                                                                                                                               |
| Syscall ABI in the threat model (ADR-0017/0022) | delete the `SYS_PUTC` row from `SECURITY.md`'s authority table | `doc-claims: the syscall ABI and SECURITY.md's authority table disagree` naming `SYS_PUTC(2)`, then exit 1 — the set is compared both ways, so an invented row fails too |
| `El0Session` field offsets                                       | insert a field before `user_ttbr`                                                                                                      | eight `offset_of` assertions red at compile time, each naming its field and its expected offset. The assembly does not actually drift — its offsets are `.equ` symbols derived from the same struct — so this is a tripwire on an _unintended_ reorder rather than the mechanism keeping the two in agreement                                                                                                                                                                                                                           |
| Stale `#[allow(dead_code)]`                                      | convert all thirteen to `#[expect(…, reason = …)]`                                                                                     | three came back _unfulfilled_: `TrapFrame`, `frames::alloc` and `frames::free` have had consumers for milestones while an attribute went on calling them dead. `allow` is silent forever; `expect` warns the moment the deroga stops being needed, which is the only difference and the whole reason to prefer it                                                                                                                                                                                                                       |
| Scaffolding in the product image                                 | pull `demos` back in through an inner `mod` with `#[path]`, so it compiles without the feature                                         | **Twice green before it was right.** v1 grepped `llvm-nm` for `bootstrap::demos` and reported clean with 4 KiB of demo code in the image — release LTO renames and inlines, so the module path is not in the symbol table. v2 listed six console markers by hand and passed the same leak, because the leaked function's output was not among the six. v3 derives every literal from `demos.rs`, validates each against the image that _has_ the oracle, and catches it: `'el0: SVC ok  imm=0' is in an image built without the oracle` |
| EL0 program encoding                                             | change the `tbnz` offset in `encode_pl011_rx_poll_exit` from 4 to 3                                                                    | `tbnz w1, #4, #12` against the intended `#16` — the branch target, in the disassembly, beside the assembly it is meant to be. Without the test the same mistake produces `rx poll unexpected putcs=…` on a board and reads like a kernel bug                                                                                                                                                                                                                                                                                            |
| The assembler is missing                                         | shadow `llvm-mc` with a command that exits non-zero                                                                                    | the test **fails** rather than skipping. `make no-simd` once reported `clean` having disassembled nothing, and that lesson is in this helper's doc-comment                                                                                                                                                                                                                                                                                                                                                                              |
| Coupling a test to a tool's _output_ format                      | push the first version, which disassembled and compared mnemonics                                                                      | **CI, on the first push**: the runner's `llvm-mc` prints a `.text` directive the development machine's does not. Local green, remote red, and the fix was not to filter the directive — it was to invert the direction. Disassembly output is a rendering; assembly input is a language. The intended text now goes through the assembler and the comparison is on bytes                                                                                                                                                                |
| Doc symbol paths                                                 | put `arch::mmu::EARLY_L1` back into `docs/mmu.md`                                                                                      | `doc-symbols: EARLY_L1 lives in src/mm/early.rs, which is not a module 'mmu'`. This is the sentence F23 left behind for a day: the finding was that board topology does not belong in `arch`, and the document explaining the map still put it there. Asking only whether `EARLY_L1` exists would have passed                                                                                                                                                                                                                           |
| Scheduler model, idle requeue                                    | make `Switch::Yield` requeue everything _except_ idle                                                                                  | `invariant broken after step 2: idle is not current and nothing is queued` with the counter-example `[Admit, Switch(Yield)]`. Two operations, and nobody would have written that test — the first version of the invariant asserted `state(IDLE) == Ready`, which the mutation satisfies while idle sits outside the queue. The model found the _specification_ too weak before it found anything about the code                                                                                                                        |
| IPC model, generation check                                      | drop `ep.generation != cap.generation()` from `Table::lookup`                                                                          | `diverge at step 2 — Send(Stale): reference says Err(BadCap), table says Ok(None)`, counter-example `[Create, Send(Stale)]`. This is the check `SECURITY.md` calls latent: no kernel path mints a stale handle, so nothing exercised it until the model offered one at every step                                                                                                                                                                                                                                                       |
| IPC model, full mailbox                                          | `mbox.len == DEPTH` → `mbox.len > DEPTH`                                                                                               | `diverge at step 4 — reference says Err(Full), table says Ok(None)` with `[Create, Send, Send, Send]`. The off-by-one that lets a bounded queue grow by one                                                                                                                                                                                                                                                                                                                                                                             |
| Image declares a feature set it does not have                    | make the headless banner claim `debug-display`                                                                                         | `boot-check: FAIL — image says debug-display, but the panel never came up`. Checked in both directions: an image claiming the panel must bring it up, one claiming headless must not touch it. Neither half alone is enough — each is satisfiable by a lie                                                                                                                                                                                                                                                                              |
| Console denied by default (ADR-0017 §3)                          | grant `CONSOLE_SLOT` to the agent that is meant to lack it                                                                             | the refusal line disappears and the byte `X` appears on the console. Both halves are asserted: the boot check fails if the denial line is missing _and_ if the denied agent's byte shows up                                                                                                                                                                                                                                                                                                                                             |
| `SessionEnd` swallowed (ADR-0018 §4)                             | read `s.end` and drop it                                                                                                               | `error: unused agent::SessionEnd that must be used`, carrying its own note — _the creator decides what happens to a faulting agent; the kernel only ended its session_. Under `-D warnings` that is a build failure, which is the whole point                                                                                                                                                                                                                                                                                           |
| Creator survives its agent's fault                               | remove the `creator alive after fault` line                                                                                            | `boot-check: FAIL — the creator did not survive its agent's fault`. One line saying "it faulted" would have hidden the two claims that matter: the creator kept running, and so did its peer                                                                                                                                                                                                                                                                                                                                            |
| Orphaned trait behind a feature                                  | remove the attribute from `SpiDevice` and build `--features debug-display`                                                             | `trait SpiDevice is never used`. It has an implementation (`ExclusiveDevice`) and no caller in any configuration. ADR-0010's _requirement_ — must not bit-bang CS — is satisfied by `with_bus`; only a sentence beside it, saying short ops use `SpiDevice::write`, stopped describing anything. [ADR-0020](adr/0020-spidevice-contract-without-a-caller.md) retracts the sentence and keeps the trait as the contract ADR-0009 adopts                                                                                                  |
| Slot bound (`cap::from_slot`)                                    | `slot >= caps.len()` → `slot > caps.len()`                                                                                             | two host tests red, both by index-out-of-bounds: the last-slot test and the empty-table test. The bound is the whole of slot-indexed authority — one past it is an agent reading a word of someone else's table                                                                                                                                                                                                                                                                                                                         |
| EL0 authority refusal                                            | run the boot with the agent that names slot 1 removed                                                                                  | `boot-check: FAIL — EL0 agent was not refused a slot it does not hold`. The refusal is on the _good_ path on purpose: a protection nobody watches fire is an assumption                                                                                                                                                                                                                                                                                                                                                                 |
| Payload crosses EL0 → EL0                                        | drop the `mov x0, x2` from the receiving agent, so it prints its status instead of the message                                         | `boot-check: FAIL — the received payload was not printed by the receiving agent`. Without the move the agent prints a zero and proves only that it resumed                                                                                                                                                                                                                                                                                                                                                                              |
| Boot check vs a binary log                                       | the same mutation, before `-a` was added to every grep                                                                                 | `FAIL — task output not interleaved`, naming the wrong assertion entirely: the agent's zero byte made `grep` treat the log as binary and stop matching. An agent can now `SYS_PUTC` any byte, so this stopped being hypothetical                                                                                                                                                                                                                                                                                                        |
| Authority counter vs a full mailbox                              | five EL0 sends into a four-deep mailbox                                                                                                | `refuse count=2 full=1` — the fifth send is `full`, and the authority count does not move. This is what the counters are separate for                                                                                                                                                                                                                                                                                                                                                                                                   |
| Authority count survives later traffic                           | host test: note a refusal, then a successful send + recv                                                                               | the count stayed at 1 rather than being erased. It _was_ erased before this slice — see below                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ADR status, three copies                                         | stamp `accepted` in ADR-0017's frontmatter alone, then run `xrefs`                                                                     | both other copies named in one run: `status is 'accepted', the index says 'proposed'` and `architecture.md does not mark it (**accepted**)`. Accepting an ADR means moving three files, which is exactly the shape that goes stale by attention                                                                                                                                                                                                                                                                                         |
| README module map                                                | run the new `doc-claims` check against the Layout block as it stood                                                                    | twenty of `kernel-core`'s twenty-five modules named as missing, plus `src/agent` — the agent shell, this project's central concept, absent from its own map — and `time.rs` listed as `time/`                                                                                                                                                                                                                                                                                                                                           |
| Board addresses inside the ISA tree                              | `const PERIPHERALS: u64 = 0xC000_0000;` and `RAM_TOP = 0x8000_0000` put back into `arch/aarch64/mmu.rs`                                | `arch-board-free: … names 0xC000_0000, a physical range base`, both lines, exit 1. This is F23, which stayed open for two days with `make layering` one directory away — that gate sees imports, and the other way to know a board is to write its addresses out by hand                                                                                                                                                                                                                                                                |
| UART RX handover order                                           | swap the two steps in `RxLine::plan_suspend`, then in `plan_resume`                                                                    | five tests red for the first, two for the second, each naming the exact step: `step 0 (ClearView) left the line armed with no view`. This is the defect a review found by reading — the window is an instruction pair wide and the boot check types nothing — and until `kernel_core::rxline` existed the only evidence it was fixed was a hardware boot nobody re-runs                                                                                                                                                                 |
| Boot check, host starvation                                      | `systemd-run --user --scope -p CPUQuota=8%` around the boot check, the level at which `timer: MISSED` first appears                    | `boot-check: INDETERMINATE — … the emulator got 0.07 cores of host CPU over 15s`, exit 3. The same script on an idle host reports `2.97 cores` and passes; with the assertion rewired to a line that is always present it reports `FAIL — … the emulator had the CPU to meet them`. All three outcomes seen, which is what makes the third one a verdict rather than a comment                                                                                                                                                          |
| No-SIMD guard, tool absent                                       | `make no-simd OBJDUMP=llvm-objdump-does-not-exist`                                                                                     | `no-simd: FAIL — refusing to report clean`. Before the check, the same run printed `no-simd: clean`: an empty pipeline made `grep .` fail and `!` inverted that into success, so the gate passed having disassembled nothing                                                                                                                                                                                                                                                                                                            |
| No-SIMD guard, FP present                                        | build the same tree for `aarch64-unknown-none` (hard float)                                                                            | `error: FP/SIMD registers found`, on `v0`. The image carries 9 scalar `h` registers the earlier `[qv]` pattern ignored — on this tree they share lines with `v`, so the widened pattern adds coverage for a class (`fmov d0, x1` with no vector register) rather than a detection                                                                                                                                                                                                                                                       |
| Board feature guard                                              | `cargo build --no-default-features`                                                                                                    | `no board selected — enable a board-* feature`; `make board-guard` asserts the refusal names the feature rather than cascading about a missing `bsp::board`                                                                                                                                                                                                                                                                                                                                                                             |
| `SCTLR_EL1` RES1 bits                                            | read the register back after boot and mask bits 11/20/22/23/28/29                                                                      | `SCTLR probe: 0x1005  RES1 set=0x0 of 0x30d00800` — six RES1 fields cleared by `msr sctlr_el1, xzr` and never restored. After the fix the same probe reads `0x30d01805`. QEMU only: the board has not been measured                                                                                                                                                                                                                                                                                                                     |
| `SCTLR_EL1` RES1, as a bring-up gate                             | restore `msr sctlr_el1, xzr` and boot `--features bringup`                                                                             | `SCTLR_EL1=0x1005 RES1=0x0/0x30d00800` then `selftest: FAIL SCTLR RES1`. The one-off probe that found this is now a gate: only _missing_ bits fail, because a part that forces its own RES1 bits is equally correct and worth knowing about                                                                                                                                                                                                                                                                                             |
| No-SIMD guard, false positive                                    | add `ldr x0, =0x30d00800` to `boot.s` (a literal pool entry, no FP at all)                                                             | `error: FP/SIMD registers found`, pointing at `.word 0x30d00800` — `objdump` prints a literal pool's raw bytes even under `--no-show-raw-insn`, and the byte `d0` reads as the register. Widening the pattern to scalar FP is what made data sections start to matter; the earlier `[qv]` pattern could not hit it because `q` and `v` are not hex digits                                                                                                                                                                               |
| GIC `enable` ordering                                            | reviewed against its own comment                                                                                                       | no red output — this one was found by reading. `GicV2::enable` masked the line _fourth_, after reprogramming group, priority, target and trigger, while the comment beside it said mask first and gave the right reason: with `enable_gic=1` the firmware has already programmed the distributor (ADR-0004), so a line can arrive live. No gate covers interrupt-controller programming order                                                                                                                                           |
| IPC refusal counters, split                                      | the M4 gate asserted on a number covering three different things                                                                       | no red output. `ipc: refuse count=1 full=0 state=0` now separates an authority violation from a full mailbox and from a dead endpoint; the gate asserts the first is non-zero and the other two are zero. Before, filling a four-deep mailbox would have satisfied the forgery assertion                                                                                                                                                                                                                                                |
| Pre-MMU path, direct branch                                      | `b switch_ttbr0` added to `_start`                                                                                                     | `_start calls 'switch_ttbr0': the pre-MMU window now includes code this check does not inspect`. The extractor harvested only `bl`, so a direct tail branch was neither audited nor refused and the gate printed clean having walked past it                                                                                                                                                                                                                                                                                            |
| Restore-to-Pi-OS backup                                          | make the backup directory unwritable and run the copy                                                                                  | `could not back up … refusing to overwrite it`, exit 1. It was `cp … \|\| true` followed by the overwrite regardless — a failed backup destroyed the Harbor image with no copy anywhere, in the one script reached when something has already gone wrong                                                                                                                                                                                                                                                                                |
| Cross-references                                                 | break a markdown link, cite an ADR number that does not exist, flip one status in the ADR index                                        | each named with its file: `links to 'verificaton.md', which does not exist`; `… is cited but no docs/adr/…-*.md exists`; `status is 'accepted', the index says 'proposed'`. All four classes were already correct — by attention, which does not survive a rename. The mutation's own number is left out of this row on purpose: writing it here makes this table a citation, and the gate is right to say so                                                                                                                           |
| IPC waiter slot                                                  | let `park` overwrite the waiter instead of refusing                                                                                    | `a_second_waiter_is_refused_not_swapped_in` fails. Until this branch the only oracle for the whole IPC path was one `grep` over a boot log                                                                                                                                                                                                                                                                                                                                                                                              |
| IPC refusal counters, as tests                                   | count a full mailbox as an authority violation                                                                                         | two tests fail, including `a_full_mailbox_refuses_without_touching_the_authority_count` — the defect the M4 gate could not see                                                                                                                                                                                                                                                                                                                                                                                                          |
| Capability generation                                            | drop `ep.generation != cap.generation()` from the lookup                                                                               | `a_forged_capability_is_refused` and `a_stale_handle_from_a_recycled_slot_is_refused` both fail. Product path now also exercises stale handles after real `revoke_channel` (ADR-0032); host tests `revoke_*` and boot-check `ipc: release stale refused`                                                                                                                                                                                                                                                                                 |
| Parked stack, as tests                                           | overwrite the parked slot instead of handing it back; then stop parking on exit                                                        | `skipping_a_collection_point_is_counted_not_silent` fails on the first; four tests fail on the second, including `an_exit_into_a_task_that_has_never_run_still_parks` — the P0-2 ordering, which no boot performs and which nothing could drive before                                                                                                                                                                                                                                                                                  |
| Slot reuse before collection                                     | let `admit` hand out a slot whose stack is still parked                                                                                | `a_slot_whose_stack_is_still_parked_is_not_handed_out` fails. A case the old design avoided by accident — it detached the stack on exit — and that became reachable when the stack was left attached to its slot                                                                                                                                                                                                                                                                                                                        |
| User-window text bound                                           | widen `bound_text_write` back to `pages * frame`                                                                                       | two tests fail, including `a_write_past_the_text_page_is_refused_even_though_the_window_is_bigger` — the P0-3 defect, where every offset in the window looked legal while the write went to page 0's physical address alone                                                                                                                                                                                                                                                                                                             |
| User-window offset overflow                                      | `checked_add` back to a wrapping add                                                                                                   | `an_offset_that_would_overflow_is_refused_not_wrapped` fails: `usize::MAX + 1` wraps to zero and reads as a legal write at the start of the page                                                                                                                                                                                                                                                                                                                                                                                        |

## Bounded exhaustive model checking (2026-08-07)

Mutation testing asks _do the tests notice a change_. This asks a different
question: _does the implementation agree with a statement of what it should do,
over every sequence of operations up to a bound_. Two files, no dependencies,
public API only, inside `make test` — so inside `make check`.

### What is bounded, stated before what was found

`crates/kernel-core/tests/model_sched.rs` — `Tasks<3>` (idle + two workers),
every sequence of at most 7 operations over an 8-symbol alphabet: **2 396 745
sequences in 0.45 s**. Five invariants after every step, all through the public
API.

`crates/kernel-core/tests/model_ipc.rs` — `Table<2, 4, 2>`, every sequence of at
most 6 operations over 13 symbols: **5 229 043 sequences in 1.7 s**. Not
invariants but a **reference implementation**: fifty lines that say what a
bounded queue with one waiter slot does, compared against the real table on
every observable — the exact `Ok`/`Err` variant, the message returned, the task
id handed back for waking, and all three refusal counters. Plus conservation at
the end of every sequence: what is drained equals what the reference still
holds, in order.

No state deduplication in either: sequences replay from scratch, so the search
cannot prune a path a coarse fingerprint would have merged. It costs replay time
and buys soundness within the bound.

**This is not a proof.** It is exhaustive on a small instance to a chosen depth,
and the step to `Tasks<14>` and `Table<8, 16, 4>` is an _argument_ — none of the
rules mentions the number of slots or mailboxes except through the constant the
model carries as a parameter — not a theorem. It says nothing about `src/`'s
`unsafe`, about the assembly, or about concurrency, of which there is none.

### The first thing it caught was the specification, not the code

The scheduler invariant was written as _"idle is `current` or `Ready`"_. Under a
mutation that stops requeueing idle on yield, the model **passed**: idle stayed
marked `Ready` while it had left the run queue. `State::Ready` is a field; queue
membership is not observable through `Tasks`, and the two had been conflated.

The property is observable by consequence — if idle is not running then idle
itself is queued, so something is always ready — and with that line added the
same mutation dies in two operations: `[Admit, Switch(Yield)]`.

That is the useful failure mode of this technique. It did not find a kernel bug;
it found that the thing being asserted was not the thing being claimed, which is
the error a hand-written test cannot report because a hand-written test only
visits states someone already believed in.

### What it retires

Three of the ten justified mutation survivors live on
`Tasks::switch`'s `Ok(None) if current != IDLE` guard, and the code comment there
says _"no test can honestly cover it"_. That remains true of chosen scenarios.
It is no longer the whole story: the invariant the guard protects is now checked
over every reachable state of `Tasks<3>` to depth 7, so the branch is
**unreachable by exhaustion within the bound** rather than unreachable by
argument. The survivors stay in the baseline; their justification is stronger.

The six `!mbox.live` survivors are the same shape, and the model says the same
thing about them: no sequence over the public API reaches a dead mailbox,
because nothing releases an endpoint.

`SECURITY.md` lists _"stale-handle check is latent"_ among the residual risks.
It is less latent now: a stale `CapId` — same index, previous generation — is in
the alphabet and is offered at every step of all five million sequences, and
removing the generation comparison from `lookup` is caught in two operations.
The kernel still never mints one; the check no longer goes unexercised.

## Mutation testing: what the tests actually cover (2026-08-06)

The table above is hand-curated, and that is its limit: it records the checks
someone thought to break. It says nothing about the other hundred and fifty
tests, which are known to pass and not known to cover anything.

`cargo-mutants` settles that mechanically. It rewrites one expression at a time
— an `||` into an `&&`, a `+=` into a `-=`, a match guard into `false` — and
reports which mutations the suite fails to notice. It is a tooling dependency
only: nothing enters the kernel's dependency graph.

Run over the three modules that carry the authority and scheduling logic:

```
cargo mutants -p kernel-core --file '**/ipc.rs' --file '**/tasks.rs' --file '**/layout.rs'
```

**First run: 129 caught, 23 missed.** The score is not the useful part; the
survivors are. Four things came out of it that no amount of reading would have:

| Survivor                                           | What it meant                                                                                                                                                                                                                                                                                                                                                                               |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Region::is_write_execute -> false`                | **The serious one.** W^X is one of the three protections this project claims, and every test asserted that good regions are _not_ W+X — all of which pass just as well with the check hard-wired to "no". There was no positive test: nothing had ever watched the check recognise a violation. It is precisely the doctrine this document opens with, applied to a test instead of a gate. |
| `refusals.state += 1` on the second-waiter path    | The refusal was asserted, the counter beside it was not. The counters are what the boot oracle reads, so an increment that stopped incrementing would be reported as a clean boot.                                                                                                                                                                                                          |
| `current == Self::IDLE` guards, mutated to `false` | Both were only ever exercised from idle, which cannot tell a guard from a constant.                                                                                                                                                                                                                                                                                                         |
| `Tasks::withdraw`                                  | Never called by anything. Written for symmetry during the extraction, and nobody noticed because a dead function passes every test. Removed rather than tested.                                                                                                                                                                                                                             |

Nine tests were written against the survivors, and one of them was itself wrong
in a way only the second run exposed: the alignment test used
`guard: (0x1008, 0x2000)`, which is _also_ a guard shorter than a page, so it
was refused by an earlier check and never reached the alignment chain at all.
It passed, and proved nothing. Rewritten to isolate each of the three terms, it
goes red under the mutation as it should.

**Final: 142 caught, 9 missed, 16 unviable — 94% of viable mutants.**

The nine survivors are all the same shape, and none of them is worth a test:

| Site                                                     | Why it survives                                                                                                                                                                                                                                                   |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Table::{send,try_recv,park}`, the `!mbox.live` arms (6) | `live` never returns to `false`: no endpoint is ever released, so a lookup that resolves cannot resolve to a dead mailbox. The arm is unreachable until release-and-reuse exists.                                                                                 |
| `Tasks::switch`, `Ok(None) if current != IDLE` (3)       | Idle is always exactly one of _current_ or _queued_ — popped when it runs, requeued when it yields, and forbidden to block or exit. So a worker asking for the next task always finds at least idle, and `Ok(None)` only ever arrives when idle itself is asking. |

Both are guards on invariants stated elsewhere in the same file, kept because
those invariants are the kind that a later change breaks quietly. A test that
reached them would have to break the invariant first, which would be testing the
test. Recording them here is the honest alternative, and it is the same
convention this document already uses for gates that cannot exist.

### Second run, after M7 slice 2: 214 caught, 10 missed, 1 timeout

`cap` and `syscall` joined the file list when EL0 gained the authority ABI, and
the run grew from 152 mutants to 256. The count of survivors moved by one and
its shape did not:

- the six `!mbox.live` mutants are the **same** arms as before. Only the operator
  cargo-mutants chose moved, from the condition to the `refusals.state += 1`
  inside it. An unreachable branch stays unreachable however you mutate it.
- the three `Tasks::switch` guards are unchanged.
- **one new survivor, and it is _equivalent_ rather than untested:**
  `CapRights::SEND = Self(1 << 0)` mutated to `1 >> 0`. Both are 1. No test can
  distinguish them because there is nothing to distinguish — this is a mutant
  that should not be counted against coverage, and the baseline says so in
  those words rather than absorbing it silently.

Two survivors from the first `cap` run **were** real gaps and were killed rather
than justified: `CapRights::RECV = 1 << 1` mutated to `1 >> 1` (which makes
`RECV` the empty set — and an empty right is contained by everything, so every
check against it passed), and `CapRights::union` mutated from `|` to `^` (which
agrees with union on the disjoint `SEND`/`RECV` pair and silently revokes a
right granted twice). The tests that kill them assert the bits are _set_, and
use overlapping rights, which the existing tests never did.

The new `cap::from_slot` and the extended `syscall` produced **no survivors**.

**`make mutants`** runs it, over `ipc`, `tasks`, `layout`, `irqtable`, `rxline`,
`reset`, `cap`, `syscall`, `prog` and `manifest`. Not wired into `make check`:
a full run is 316 mutants and well over twenty minutes on a loaded machine,
and the value is in reading the survivors rather than in a threshold. It belongs
where ADR-0001 puts the multi-role review — before a milestone that moves a
boundary.

The target compares against the ten justified survivors above rather than
against zero, because `cargo-mutants` exits non-zero whenever anything survives
and a target that is red every time is a target nobody runs. More survivors than
the baseline fails and prints them; fewer says so and asks for the baseline to
be lowered, since a stale one hides the next regression.

`kernel_core::reset::partition` contributes one _timeout_: its loop counter
mutated to a no-op never terminates. That is a detected mutant and not a
surviving one — the suite hangs rather than passes — and the baseline counts it
separately.

The modules added since the first run — `irqtable` and `rxline` — produced **no
survivors at all**, which is the useful measure of tests written with the
mutants in mind rather than found by them afterwards.

### Third run, after the loader and the park: 274 caught, 10 missed, 1 timeout

`manifest` joined the file list — it is the code that decides whether an agent
may receive authority, and it had never been mutated — and `layout` was
re-examined because `UserWindow` grew `text_pages` the same day. The run went
from 256 mutants to 316.

**The survivor set did not move.** Still the same ten: one equivalent
(`CapRights::SEND = 1 << 0` mutated to `1 >> 0`, both are 1), six `!mbox.live`
arms guarding an endpoint that release-and-reuse will one day make reachable,
and three `Tasks::switch` guards. Sixty more mutants caught and not one new gap
— which is the useful reading, because `manifest` and the reworked `layout`
were written with these tests in mind rather than tested afterwards.

#### The one thing it did find, and why it is not in the survivor list

`manifest::bind` contributed a **second timeout**: its `slot += 1` mutated to
`slot *= 1`, which pins the index at zero and hangs the suite. A timeout is a
*detected* mutant — a hanging test is not a passing one — so the honest options
were to raise the timeout baseline from 1 to 2, or to remove the counter.

The counter went. `bind` now walks `entry.slots.iter().enumerate()`, which has
no `+=` to mutate, and a scoped re-run of `manifest.rs` alone reports **15
caught, 1 unviable, zero missed, zero timeouts**. So the baseline stays 10 and 1
rather than growing to accommodate a shape that did not need to exist.

That is the difference worth naming: raising a baseline records a weakness,
rewriting the loop removes one. The first is sometimes right — the six
`!mbox.live` arms are unreachable and stay — and this was not one of those
times.

#### What mutation testing cannot reach here

`cargo-mutants` runs `-p kernel-core`. Everything in `src/` is outside it,
because it is not host-testable — which means the *kernel-side* half of some
claims has no mutation coverage at all. Concretely, after ADR-0022: the table's
`Busy` refusal is covered (host tests and the bounded model), but the mapping
`RecvError::Busy → Status::Busy` in `src/agent/mod.rs` is two lines nothing
mutates and nothing on the boot path reaches, because nothing creates a second
waiter. Named here rather than left to be inferred from a green run.

## The refusal counter that erased itself (2026-08-06)

Found while adding `SYS_SEND`, by reading a boot log that did not add up: two
distinct authority refusals had happened and the console said `count=1`.

`REFUSED_AUTHORITY` had **two writers with different semantics**. The kernel-side
holder check (`sched::current_holds`, which the pure table cannot perform)
incremented the atomic directly; every table operation then _stored_
`table.refusals()` over it. So a caller-side refusal survived exactly until the
next successful send, and then vanished.

What makes it worth its own section is what it did to a gate. The M4 assertion
`ipc: refuse count=[1-9]` existed to prove the forger's capability check fired.
With the counter resettable, that line could be satisfied by _a different
refusal that happened later_ — and once the EL0 agent started producing
refusals of its own, it was. The gate passed while naming something it had not
verified, which is worse than failing.

The fix is that the table owns the number: `Table::note_authority_refusal` lets
the kernel report the check it alone can perform, and the atomics stay what
their doc-comment always claimed they were — mirrors, never sources. The
regression test asserts a noted refusal survives a full round trip, and the M4
gate now asserts `count=2` exactly, because "at least one" is what let two
different facts satisfy the same assertion.

Neither the host tests nor any gate would have found this. It was found by a
number in a log being smaller than the events that produced it.

## Four defects no gate caught (2026-08-05)

The table above records checks proven to work. This section records the
opposite, which is the more useful half: a multi-role review found five
correctness defects, and **`make check` stayed green through all of them**.
Four were invisible to every gate; only the fifth had a check waiting for it,
and that check had been sized against a stale constant so it never fired.

| Defect                                                                                                                                                                                                                               | Why no gate saw it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `sched::init` / `spawn` unmasked IRQs unconditionally instead of `irq_save` / `irq_restore`, re-enabling them after bootstrap deliberately left them masked on a failed `board::irq::init()`                                         | The boot-check oracle only reads a healthy boot. Nothing exercises the degraded path where the GIC never binds, so the line promising "interrupts stay masked" is never checked against `DAIF`.                                                                                                                                                                                                                                                                                                                                |
| `task_trampoline` never drained `pending_free`, so an exit followed by a never-yet-run task dropped a `TaskStack` whose `Drop` is a deliberate no-op — 20 KiB of heap and an unmapped guard page inside a live heap block, uncounted | `abandoned_stacks()` counts only stacks whose guard could not be remapped. A stack that is silently _dropped_ never reaches `release()`, so the one counter watching this class could not see it. `src/sched` has no host tests. Latent in the current boot: every spawn in `bootstrap::run` happens before the scheduler starts, so by the first exit every task has already run once and the ordering never arises. Removing the drain again leaves the boot check green — which is why the counter and its assertion exist. |
| `AddressSpace::poke_user` validated against the whole 16 KiB user window while writing from `user_base_phys`, the physical address of page 0 alone                                                                                   | Latent: every caller passes 28 bytes or fewer at offset 0. A bound that is wrong only for inputs nobody sends is exactly what a boot-log oracle cannot distinguish from a bound that is right.                                                                                                                                                                                                                                                                                                                                 |
| `console::suspend_rx` disarmed the IRQ view before masking `IMSC`, leaving a window where a byte makes the handler return without popping `DR` or writing `ICR` — an unclearable level-triggered storm                               | The window is one instruction pair wide and needs a byte to arrive inside it. The QEMU boot check types nothing during the handover, so the race has no way to happen. `resume_rx` held the mirror-image inversion.                                                                                                                                                                                                                                                                                                            |

What these share is the shape named at the top of this document: the oracle is
one healthy boot. It is strong at proving the good path stays good, and blind to
degraded paths, to bounds nobody currently exceeds, and to races too narrow to
hit by accident. The cheapest way to close the class is to move the bookkeeping
in `src/sched`, `src/mm/aspace.rs` and `src/ipc` down into `kernel-core`, where
it can be tested on the host — every one of these four lived there.

**Done, and the target that came with it was the wrong one.** `kernel_core::ipc`
took the authority surface, `kernel_core::tasks` the scheduler state machine,
`kernel_core::layout::UserWindow` the window geometry that the third defect
above got wrong, and `kernel_core::irqtable` the dispatch table whose seal
nothing had ever tested. `src/mm/aspace.rs` keeps its frame ledger and is the
remaining candidate.

The goal written at the time was "`src/` under 5000 lines". It went from 9181 to
about 8900, and chasing the rest would mean moving hardware bindings into
`kernel-core` to empty a directory — the opposite of why `kernel-core` exists.
The number was never the point. What matters is whether the _decisions_ in
`src/` are falsifiable, and that is now true of IPC authority, scheduling, user
window bounds and IRQ dispatch, and not yet of the address-space ledger or the
console RX state machine.

`sched::pending_overwrites()` was added with the second fix for the same reason:
the single-slot invariant behind `pending_free` was documented as true and was
not. The idle loop now reports it (`sched: PENDING-OVERWRITE n`) rather than the
comment asserting it.

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


## Hardware evidence: M8 console endpoint closed on silicon (2026-08-07)

M8 retires `SYS_PUTC`. Console output is `SYS_SEND` with `CONSOLE_TAG_BYTE` (0)
and the byte in `Message.a`. An EL1 `console_server` holds the recv end and
drains via `console::with_tx`. Creators call `ipc::yield_until_empty` before
report lines so agent bytes land on the wire before the creator's report line.

| Claim | Gate / evidence |
| ----- | --------------- |
| Server up | `console-server: up` — QEMU + Pi 4B |
| Product beacon | `loader: beacon ran sends=2 refusals=0` + wire `H!` before the report |
| Mute denial (oracle) | `loader: mute ran sends=0 refusals=2`; refuse count=5 |
| Console via SEND (not putc) | `el0-task: console sends=2`; `decode(2) == Unknown` |
| Product image | `make product-builds` + `make product-boot-check` |
| Payload still crosses EL0 | `*el0-ipc: got payload via EL0 recvs=1` |

**Status: done (HW)** on Pi 4B, 2026-08-07 ~15:25 host time. Transcript:
`.serial-log/20260807-152525.log` (oracle `kernel8.img` @ `ea24a24` lineage).

### Silicon excerpt (Pi 4B, PL011 @ 115200)

```
console-server: up
console: capability minted
loader: beacon loaded text=1 stack=3
loader: mute loaded text=2 stack=3
…
loader: mute ran sends=0 refusals=2
…
H!H!loader: beacon ran sends=2 refusals=0
…
el0-task: console sends=2
…
ipc: refuse count=5 full=0 state=0
*el0-ipc: got payload via EL0 recvs=1
…
ticks=10
```

`H!H!loader` is two agents printing `H!` (beacon then el0-task) before the
loader report; the adjacency claim for the barrier is that the beacon's bytes
precede `loader: beacon ran`, which they do. Idle ticks continued past 300.

## Parked-task visibility and cancel closed on silicon (ADR-0024 / 0025, 2026-08-07)

| Claim | Gate / evidence |
| ----- | --------------- |
| Parks are counted | Host tests; boot-check `sched: blocked=… block_events=…` |
| Orphan wait cancelled | Boot-check + Pi 4B: `ipc: reaped cancelled` + `ipc: cancel issued cancel_events=` |
| Waiter cleared without send | Host test `clear_waiter_drops_the_parked_slot_without_a_send` |
| EL0 status | `Status::Cancelled = 5`; SECURITY authority table |

**Status: done (HW)** on Pi 4B, 2026-08-07 ~15:59 host time. Transcript:
`.serial-log/20260807-155757.log` (oracle `kernel8.img` with reaping demos,
`e0e905e` lineage).

### Silicon excerpt (Pi 4B, PL011 @ 115200)

```
console-server: up
…
ipc: orphan spawned id=15
ipc: reaper spawned
…
H!H!loader: beacon ran sends=2 refusals=0
…
ipc: refuse count=5 full=0 state=0
ipc: cancel issued cancel_events=1
sched: blocked=0 block_events=6
ipc: reaped cancelled
…
ticks=10
```

**Later H1 slices (QEMU, not yet HW-stamped here):** last-SEND-hold auto-reap
([ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md) — boot-check
`ipc: auto-reaped cancelled`); channel revoke ([ADR-0032](adr/0032-k3-channel-revoke.md)
— `ipc: release stale refused`); EL0 `SYS_WAIT_IRQ` ([ADR-0030](adr/0030-el0-irq-capability.md)).
**Still open:** K2 timeout queue; K3 cap transfer; full H1 path on the
[completeness roadmap](roadmap.md).
