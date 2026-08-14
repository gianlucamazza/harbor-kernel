# Verification

What is checked, by what, and — the part that matters — what each check cannot
see. A gate whose blind spots are undocumented gets trusted for things it never
covered.

## How to read this file

It is an **index of evidence**, not onboarding, and it is long because
transcripts are kept rather than summarised. Nobody should read it end to end.

| If you want…                                           | Go to                                                                                                                                                                                                                             |
| ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| What each layer of checking covers **and is blind to** | [The layers](#the-layers) — the one section worth reading in full                                                                                                                                                                 |
| Why `done (QEMU)` is weaker than `done (HW)`           | [What emulation cannot catch](#what-emulation-cannot-catch-with-the-example-that-proved-it)                                                                                                                                       |
| The evidence behind one specific claim                 | Follow the link from the claim; the section headings are dated                                                                                                                                                                    |
| Where the gates are still blind                        | [Checks that have been seen to fail](#checks-that-have-been-seen-to-fail), [Four defects no gate caught](#four-defects-no-gate-caught-2026-08-05), [Mutation testing](#mutation-testing-what-the-tests-actually-cover-2026-08-06) |
| What is _done_, rather than how it was shown           | [`roadmap.md`](roadmap.md) — status lives there, not here                                                                                                                                                                         |

Just arriving at the project: the [root README](../README.md) and the
[5-minute path](README.md#the-5-minute-path) come first. This file answers
"why should I believe it", which is the fourth question, not the first.

## The layers

| Layer                                            | Runs                                                                                                                             | Covers                                                                                                                                                                                                                                                                                                                                                                     | Blind to                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Formatting (`make fmt-check`)                    | `cargo fmt --all --check`                                                                                                        | That the tree is formatted as `rustfmt` would write it, so a diff is about the change and not about whitespace                                                                                                                                                                                                                                                             | Everything about meaning. It was also the one `check:` target with no row here until 2026-08-11 — a table that claims to list every layer has to be completed by hand, because nothing compares it to the Makefile                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Mutation freshness (`make mutation-freshness`)   | `cargo mutants --list` against `docs/mutation-stamp.toml`                                                                        | That the mutable surface has not moved since the last recorded run: a new function, branch or operator changes the count, and a stale run stops being invisible (ADR-0096)                                                                                                                                                                                                 | Tests that stopped killing what was _already_ there — only a run answers that. It also cannot see `src/`, which no mutation covers at all                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Host unit tests (`make test`)                    | `cargo test -p kernel-core`                                                                                                      | Register encodings (UART, SPI, RNG200, …), allocator arithmetic, GIC index maths, region splitting, the SPSC ring                                                                                                                                                                                                                                                          | Anything that touches hardware, and any _use_ of these functions                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Miri (`make miri`)                               | Interprets only `ring::tests` and `wake::tests` with `cargo +nightly miri test --lib`                                                                                                      | Aliasing, provenance and data races in the crate's two module-scoped `unsafe` queues — the ring and wake `UnsafeCell` buffers plus their `Sync` assertions                                                                                                                                                                                                                 | Pure kernel logic outside those queues is covered by host tests/model checks; kernel-side `unsafe` touching MMIO/system registers cannot be interpreted. Targeting the two unsafe unit suites keeps the gate finite without omitting relevant unsafe coverage                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Product image (`make product-builds`)            | Builds without `oracle`, greps the image                                                                                         | Diagnostic scaffolding reaching the production surface (rule 9), by the strings the demos print — derived from `demos.rs`, so the marker set cannot drift from the code                                                                                                                                                                                                    | Scaffolding that leaks without printing anything; the symbol check is a second, weaker net                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| EL0 programs (in `make test`)                    | Assembles the intended text, compares bytes                                                                                      | That the bytes an agent runs are the instructions the doc-comment claims — `llvm-mc` assembles, so nobody transcribes hex in either direction                                                                                                                                                                                                                              | Whether the program is the _right_ one for the test; only that it is the one written down                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Bounded model check (in `make test`)             | Replays every operation sequence to a bound                                                                                      | The scheduler's invariants and the authority core's agreement with a reference implementation, over all sequences rather than chosen ones                                                                                                                                                                                                                                  | Anything outside `kernel_core::{tasks, ipc}`, anything past the bound, and any `unsafe` — the walk is over safe code                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Bring-up build (`make bringup-builds`)           | Compiles and lints `--features bringup`                                                                                          | A configuration nothing else builds, and the one you reach for when the board will not talk                                                                                                                                                                                                                                                                                | Anything the gates do not _run_ — it compiles, it is not executed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| No-SIMD guard (`make no-simd`)                   | Disassembles the linked image                                                                                                    | A build that silently regains FP/SIMD                                                                                                                                                                                                                                                                                                                                      | FP that never reaches the image                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| No-`static mut` (`make no-static-mut`)           | Greps `src/` and `crates/` for declarations and `-> &'static mut` signatures                                                     | A `static mut` reintroduced after ADR-0019 landed the last one as an `AtomicPtr`                                                                                                                                                                                                                                                                                           | Prose that names the form. The `-> &'static mut` accessor shape is now refused too (one argued exception: `el0::current`); coupling that is neither a declaration nor that shape stays review's                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| IRQ scope (`make irq-scope`)                     | Walks each `cpu::without_irqs(` region brace-by-brace                                                                            | A task switch inside a masked region — the `DAIF` pair would span it and hand the next task this task's mask. Also refuses raw `cpu::irq_save()` outside `cpu.rs` and `sched`, and — since ADR-0091 — `Mutex::lock_masked` outside `src/sched/mod.rs`, which is what keeps the switch path's hand-rolled release one file wide instead of a capability every caller has    | Indirect switches: a call that parks three frames down is invisible to a lexical check. The two allowed raw-`irq_save` regions (the primitive itself; `switch_with`) cannot be walked — they legitimately contain the switch — so they stay review's job                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Pre-MMU path (`make no-early-exclusives`)        | Disassembles `_start` and its callees                                                                                            | Atomic read-modify-write before translation is on, the path growing, and any indirect branch on it                                                                                                                                                                                                                                                                         | Nothing on that path: an edge it cannot follow is refused rather than skipped                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| QEMU boot (`make boot-check`)                    | Boots the image, asserts on the log                                                                                              | MMU activation, allocator reclaim, timer IRQ, WFI idle, unhandled interrupts, panics                                                                                                                                                                                                                                                                                       | **Memory attributes.** Also cache behaviour, real clocks, firmware state. RNG200 is not modelled on `raspi4b` — init reports `NotPresent` via `arch::probe`, not a successful FIFO read. **CI note:** Ubuntu apt QEMU (≤8.2) lacks the `raspi4b` machine; GitHub Actions wraps an Arch-packaged QEMU that includes it. Local Arch/QEMU ≥9 already has `raspi4b`. **A starved host earns no verdict** (ADR-0087): the gate measures the emulator's share of a core, and below 0.40 every assertion failure is `INDETERMINATE` (exit 3), because a boot that did not get the CPU does not get far enough for any missing line to be the kernel's. The share comes from the emulator's own `/proc` entry, or — when there is no process of ours to watch, as with the CI wrapper that `docker run`s QEMU in a container — from the host's busy delta, labelled as such. |
| Product QEMU boot (`make product-boot-check`) | Boots the shipped, oracle-free product image and asserts its composition minimum | The product path is judged only after the same 0.40-core CPU budget is established; `timer: MISSED` is a hard failure above the bar and `INDETERMINATE` below it, never an ignored signal | Pi firmware, real device attributes, and any claim beyond the composition minimum; an explicit local `ALLOW_BOOT_SKIP=1` remains a skip, not evidence |
| Panic path (`make panic-check`, `make hw-check`) | Boots a `panic-probe` image that faults on purpose (`scripts/lib/panic-oracle.sh`, shared with the hardware gate)                | The panic path as a whole, with **positive** evidence (ADR-0093): a write to a real task stack's guard page, announced first, then the trap → `record_fault` → `last_fault` → address naming → halt chain. The printed `FAR` is compared with the address the probe announced, so a stale syndrome cannot pass                                                             | Five of the six `describe_address` branches — they are selected by pure, host-tested, mutated code, so what was missing was the wiring, not the branches. Also anything after the first fault: the image halts                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Lab x86 boot (`make x86-boot-check`)             | Boots `harbor-x86.elf` under `qemu-system-x86_64` q35, asserts L0 oracle                                                         | PVH entry → long mode, COM1 16550 TX, CPUID identity line ([ADR-0071](adr/0071-h3-l0-x86-qemu-first-slice.md))                                                                                                                                                                                                                                                             | Everything past L0 (timer, sched, SMP, bare-metal laptop). Not in `make check` product path — run explicitly; status is **done (QEMU-x86)**                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Doc symbols (`make doc-symbols`)                 | Module paths in the descriptive docs                                                                                             | A sentence that names `a::b::NAME` after `NAME` moved to another module — path-aware, because the symbol usually still exists somewhere                                                                                                                                                                                                                                    | ADRs and reviews, which are dated records; anything named without a module path                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Doc claims (`make doc-claims`)                   | Compares the docs against the source for facts written twice                                                                     | The `make check` gate list, the host test count, the module lists, the ADR dates, and the **set** of syscalls in `SECURITY.md`'s authority table — a call the kernel decodes and the threat model omits is a call nobody considered                                                                                                                                        | Whether a claim is _true_, only whether the two copies agree. `4 \| SYS_RECV \| (non-blocking)` stayed green the day `SYS_RECV` learned to block: the row was there and the number was right. The _reply_ semantics (status/payload/counter per outcome) are since ADR-0060 host-tested in `kernel_core::reply`; the prose rows of `SECURITY.md` remain review's                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Layering (`make layering`)                       | Every `crate::` import edge in `src/`                                                                                            | The rules in `architecture.md`: drivers never know the board, arch never names a driver, `exception` reaches only `irq`                                                                                                                                                                                                                                                    | Coupling that is not an import — a shared constant, an agreed register value, a naming convention                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Roadmap evidence (`make roadmap-evidence`)       | Compares roadmap done-row ADR citations to this file                                                                             | A status flip that leaves no trace in the evidence index — three landed exactly that way (ADR-0050/0052/0054) before this gate existed                                                                                                                                                                                                                                     | Whether the entry is _good_ evidence; only that it exists                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Arch board-free (`make arch-board-free`)         | Greps `src/arch` for 256 MiB-aligned hex literals                                                                                | A physical range base hand-written into the ISA tree (F23's shape)                                                                                                                                                                                                                                                                                                         | A base written in decimal, or assembled from shifts and multiplications                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Xrefs (`make xrefs`)                             | Links, ADR citations, status triplication                                                                                        | A doc pointing at a moved file; an `ADR-NNNN` cited but absent; file/index/architecture status drift                                                                                                                                                                                                                                                                       | Content behind a link; `#anchors`; accepted-ADR **body** rewrites — those are governed by ADR-0058's `amended:` convention and stay review's job                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Shellcheck (`make shellcheck`)                   | Lints every gate script                                                                                                          | The gates themselves — a shell bug in a checker is a checker that lies                                                                                                                                                                                                                                                                                                     | Logic errors shellcheck cannot type                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Board guard (`make board-guard`)                 | Asserts the no-board build fails, saying why                                                                                     | Board selection silently defaulting                                                                                                                                                                                                                                                                                                                                        | —                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Debug builds (`make debug-builds`)               | Compiles and lints the dev profile                                                                                               | That configuration no longer compiling                                                                                                                                                                                                                                                                                                                                     | Anything not compiled. The ~1.2k LoC SPI/TFT island this row used to declare as a blind spot is **gone** — [ADR-0094](adr/0094-retire-debug-display.md) retired it rather than reword the admission                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Product boot (`make product-boot-check`)         | Boots the product image, asserts on the log (`scripts/lib/product-oracle.sh`, shared with `hw-transcript-check`)                 | **Composition minimum**: blob endpoint plus five-agent store (beacon+chirp+lookup+entropy+blob; separate request/reply capabilities), alongside boot identity, memory self-checks, IRQ/timer/dual-current SMP, console/name binds, non-ambient resolve grant, wire bytes, invariant beacon zeros, anomaly negatives, and oracle-string ban | Oracle demos (IPC refuse counts, EL0 session probes, density workers, peer transfer, …). Product proves the shipped path rather than every lab probe. |
| Oracle census (`make oracle-census`)             | Compares `MAX_TASKS` across source, architecture table, and documented raise; **boots the product** and reads its slot watermark | Silent `MAX_TASKS++` without updating the capacity map; measured product peak occupancy vs ceiling ratio (ADR-0085: ceiling is oracle tax, not density). Since [ADR-0098](adr/0098-slot-meter-measured.md) the peak is the largest `slots=<live>/<peak>` the shipped image printed — a missing field fails the gate rather than falling back to the constant it replaced   | Peak concurrency of the **oracle** boot (call-site count only — the demos are sequential and exit); whether a raise was _wise_. Practice: CONTRIBUTING §6; knowledge `bp-demo-ceiling-is-tax-not-density`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Hardware                                         | A Pi 4B on a serial console                                                                                                      | Everything above, for real                                                                                                                                                                                                                                                                                                                                                 | Only what you actually boot and look at                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |

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

## Evidence index by ADR

Every ADR a roadmap `done` row cites must appear in this file — `make
roadmap-evidence` enforces the set. Three flips (ADR-0050, 0052, 0054) once
landed with no row here while line "peer transfer / resolve-grant still
residual" sat 500 lines down; this table is what closes that gap. Tier is the
roadmap's claim; the string is what `make boot-check` asserts (or the transcript
section below records for HW stamps).

| ADR                 | Slice                                         | Evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ADR-0102            | Product binds a name; agent discovers `console` | Host: `may_resolve` bit 8 round-trips while bits 9..31 remain refused; the 52-byte encoder matches the `llvm-mc` oracle. QEMU: `authority: bound console`, `lookup` resolves without an initial console slot and sends `N`; oracle `noresolve` runs the same image with two per-session refusals (resolve plus denied send) and no wire output. **Pi stamp 2026-08-14, transcript `20260814-113438.log`** — product image on silicon (`src=dcc997cc`, `cpu: Cortex-A72 r0p3`, `reset: PowerOn`): `authority: bound console`, `loader: lookup ran sends=1 refusals=0`, `N` on the wire. `make hw-check` clean. |
| ADR-0103            | P2 durable storage endpoint for EL0         | Host: blob wire encode/decode tests. QEMU: product starts the EL1 service, provides and binds `blob`/`blob-reply`, and the five-agent composition records `blob: put ok` plus `blob: got`; `make product-boot-check` is the end-to-end gate. **Pi stamp 2026-08-14, transcript `20260814-113438.log`** — product image on silicon (`src=dcc997cc`): `loader: store n=5 image`, `authority: 1 blob ok`, `authority: 2 blob-reply ok`, `blob: put ok`, `blob: got`, `S`, `loader: blob ran sends=3 refusals=0`. `make hw-check` clean. |
| ADR-0104            | P3 edge-gateway network composition         | **Transport, split-ring descriptor lifecycle, EL1 packet service, and directional capability binding integrated.** `kernel_core::virtio` and `kernel_core::net` host tests cover identity/status validation, modern-feature refusal, checked descriptor/queue arithmetic, directional bounded-pool ownership, wire-token validation, malformed lengths, and reset/generation invalidation. `make qemu-virtio-check` boots AArch64 QEMU `virt` with and without `virtio-net-device`, proving DTB mapping, modern negotiation, private EL1 RX/TX buffers, `DRIVER_OK`, TX descriptor submission/completion, copy-backed TX acceptance/completion through directional endpoints, retained EL1 frame ownership, 32 slot IRQ bindings, deterministic peer RX payload delivery, service reset/recovery, and absent-device refusal. External-store packet-pool encoding and Pi4 separation are covered by the host/parser and ADR-0105 evidence gate. |
| ADR-0105 + ADR-0106 | Pi 4 GENET v5 backend boundary and design | Host: `kernel_core::genet_fdt` validates the distributed Pi 4 DTB binding, ordered MMIO translation, interrupt-parent, PHY and all DMA apertures; `kernel_core::genet` covers descriptor status, ownership, bounded rings, reset generations, queue-0 `RingProgram` (v5 word-unit start/end after descriptor RAM; a packet buffer is never a ring base), clause-22 `MdioTxn` plus absent PHY-ID refusal, and `PhyLink` BMCR/BMSR classification that requires the binding's `rgmii-rxid`. The `genet_emulation` tests are deterministic non-hardware runs: programmed queue 0 plus a PHY identify → reset-cleared → link-up path and an absent-ID refusal. AArch64 `src/drivers/genet.rs` can write that program, run `init_phy`, and enable queue 0 only after `DmaPhase::Programmed`; `invalidate_dcache_poc` is an AArch64 PoC operation, not a `qemu-virt` feature. It is not selected by `board-rpi4`. A 2026-08-14 Pi 4B oracle boot stamp now proves the board baseline, and the product boot `20260814-113438.log` (`src=dcc997cc`) likewise leaves `authority: network vocabulary VACANT`. No GENET-capable QEMU device, bound MMIO backend, or Pi 4 NIC capture exists; GENET remains unclaimed. |
| ------------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| ADR-0027 + ADR-0029 | K6 external store format + in-image placement | QEMU inject path in `make boot-check` (`loader: builtin` fallback when the store is empty); product store `n≥2` in `qemu-product-boot-check.sh`. **Pi stamp 2026-08-11, transcript `20260811-122821.log`** — `loader: store n=2 image` on silicon, both agents loaded and run, plus the `loader: builtin` fallback on an earlier boot in the same capture. The stamp was taken for [ADR-0088](adr/0088-product-home-cpu.md) and paid K6 and P1 in passing; the roadmap rows said `done (QEMU)` for another day until someone read the transcript for what else it showed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ADR-0088            | Product `home_cpu` / loader pin               | Host parse/pack `home_cpu`; `make product-boot-check`: `loader: beacon … home=0`, `loader: chirp … home=1`, both ran + wire bytes; sticky admit via `spawn_with_slots_on`. **Pi stamp 2026-08-11, transcript `20260811-122821.log`** — the product image on silicon: `loader: store n=2 image` → `beacon loaded … home=0` / `chirp loaded … home=1`, both ran, both agents' bytes on the wire. `make hw-check` clean against the **product** assertions (`scripts/lib/product-oracle.sh`), which this capture is the first to exercise                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ADR-0089            | K5-B pair collapse design                     | **Design only** — no code gate; residual language in roadmap/architecture cites 0089; code deferred until §3 trigger + successor ADRs                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ADR-0090            | K10 force-exit Running                        | `force-kill: requested` / `child forced` / `slot empty` in `make boot-check` (EL1 Running + trampoline/agent `SessionEnd::Forced` path); host N/A. **Pi stamp 2026-08-11, transcript `20260811-122821.log`** — one oracle boot on Pi 4B silicon (`cpu: Cortex-A72 r0p3`, `src=1ed04fbe`, `boot=21 from=Previous`), `make hw-check` clean: `force-kill: requested events=1` → `child forced` → `slot empty` on silicon, which QEMU had shown but the board had not                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ADR-0098            | Slot meter measured, not remembered           | Host: `live_count` / `peak_slots` over admit, exit, block, refusal, CPU 1's idle and the parked slot — the watermark asserted **exactly**, not "at least". QEMU: `make product-boot-check` asserts `slots=<live>/<peak>` on the shipped image's invariant beacon (seen red against a transcript with the field stripped, and against a doctored peak below the live count); `make oracle-census` boots the product and reads the peak — **measured `slots=3/5`**, the same 5 the constant claimed, now earned. Seen red both ways: no field → refuses to guess, and a ratio that puts the peak at the ceiling → names the ADR-0085 §3 slot wall. **Pi stamp 2026-08-11, transcript `20260811-200241.log`** — the product image on silicon (`src=045a7d64`, `cpu: Cortex-A72 r0p3`, `reset: PowerOn`) prints `invariants: … slots=3/5` on every tick report: the same live count and the same watermark QEMU measured, from the board. `make hw-check` clean against the product assertions. The row said HW was not required — it rode the next stamp, which is the one taken to watch this field. **Pi stamp 2026-08-14, transcript `20260814-113438.log`** — the five-agent product (`src=dcc997cc`) prints `slots=4/9`: peak 9 while beacon, chirp, lookup, entropy and blob overlap the two idles, the console server and the blob service; live 4 after those agents exit. QEMU `oracle-census` on the same composition measures peak **8** (`entropy` refused — no RNG200). The meter moved with the composition; the ceiling is still 57                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ADR-0099            | Composition vocabulary (declared `held`)      | Host: a vacancy at 0 does not move 1 — the shift bug seen red by hand, reporting `[Some(cap), None]` where the composition meant `[None, Some(cap)]`; `Duplicate`/`Full`/`OutOfRange`/`AlreadyProvided`; `HeldVacant` and `NoSuchCapability` asserted as **distinct** refusals; the ADR-0021 boundary test ported to `Option`. QEMU: `authority: 0 console ok` in `make product-boot-check` (seen red with the line filtered out and with it turned to `VACANT`), `VACANT` among the negatives (seen red on an injected second position), and the store agents still load and run. Gate: `make vocabulary-sync` compares the kernel's declared indices against the packer's table — seen red on an index drift and on a position the kernel never declared **Pi stamp 2026-08-12, transcript `20260812-110422.log`** — the product image on silicon (`src=60e5a178`, `cpu: Cortex-A72 r0p3`, `reset: PowerOn`, Pi 4 Model B Rev 1.5) prints `authority: 0 console ok` before the loader runs, then loads and runs both store agents (`refusals=0`, `?` and `H!` on the wire). `make hw-check` clean. The vocabulary is now a line the board prints, not only one QEMU asserts                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ADR-0100            | Device windows (named by index)               | Host: `Set<T>` generic with every ADR-0099 property intact for both instantiations (a window vacancy at 0 does not move 1; independent ceilings; provide-twice refused); `NoSuchWindow` and `WindowVacant` asserted **distinct**; the pa from the vocabulary and the va from the entry, proven by resolving index 1 while index 0 holds a different page and different rights; the format tests — window+va round trip, `WINDOW_NONE` with a non-zero address refused, unaligned address refused, the device word's high bits refused, and v1 refused rather than read with defaults. QEMU: `authority: windows 1 declared` in `make product-boot-check` (`0` until [ADR-0101](adr/0101-composed-driver-agent.md) declared one); `loader: nowindow refused — names window 3 of 1` in `make boot-check`, with the negatives that a refused entry was neither loaded nor ran. Gate: `make vocabulary-sync` compares a second table — **seen red** on an index drift (kernel `rng 0`, packer `rng 1`) and **seen green** when they agree. The first version of that comparison was seen to be unable to fail — it required `^NAME ` and so parsed nothing out of `WINDOWS: dict[str, int] = {}` — and the anchor was fixed before the gate was believed. **Pi stamp 2026-08-13, transcript `20260813-101713.log`** — the row waited for a window to exist, and [ADR-0101](adr/0101-composed-driver-agent.md) declared one: `authority: windows 1 declared` and an entry resolving index 0 to the board's `RNG200_BASE` on silicon (`src=25c44332`, `cpu: Cortex-A72 r0p3`, `reset: PowerOn`). `make hw-check` clean. **Not covered:** no gate stops a `pa` being added back to the wire format — the format tests pin the current record and a new field would break none of them; what stands against it is structural (the type holding a `pa` lives in `held`, filled only from the BSP) and named in the ADR's gate section for a reviewer                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ADR-0101            | First composed driver-agent (`entropy`)       | Host: the encoder's 44 bytes against the `llvm-mc` oracle — a device load, a bit test and two _different_ bytes on the two branches, so an encoder that dropped the load changes what reaches the wire; `bind_window` resolving `entropy`'s entry against a provided window and against a vacant one. QEMU: the expectation is derived from the board rather than from a list of boards — the oracle reads the transcript's own `rng200:` line and requires `authority: 0 rng …` to agree with it; on `raspi4b` that is `rng200: unavailable (NotPresent)` → `authority: 0 rng absent`, `loader: entropy refused — window rng is VACANT`, and the negatives that `entropy` was neither loaded nor ran. `authority: 0 rng FAILED` fails on either board. Also `authority: windows 1 declared` and `loader: store n=3 image`. Gate: `make vocabulary-sync` now compares a **non-empty** `WINDOWS` table (`rng 0`) instead of an empty one against an empty one; `product-image.sh` round-trips every packed store through `inspect-store.py` before injection — **seen red** by pointing its `SUPPORTED` back at `(1,)`. **Pi stamp 2026-08-13, transcript `20260813-101713.log`** — the positive path, which cannot be green anywhere else: `rng200: ok word=0xfbf39375`, `authority: 0 rng ok`, `loader: entropy loaded text=1 stack=3 home=0`, `?H!R` on the wire and `loader: entropy ran sends=1 refusals=0` on silicon (`src=25c44332`, `cpu: Cortex-A72 r0p3`, `reset: PowerOn`). The word differs from the previous boot's `0x425ac51e`, which is what a real RNG200 does. A driver-agent that arrived in a store, was granted a page by index arithmetic, read the device and reported what it read; `make hw-check` clean. The invariant beacon reads `slots=3/6` — the peak moves from the QEMU-measured 5 because on this board the third agent actually runs, and ADR-0098's measurement stays correct for the boot it measured. The first attempt (`20260813-100737.log`) failed on the oracle rather than on the board: `product-oracle.sh` required `Rloader: entropy ran`, the interleaving QEMU produces, while on silicon the byte landed on beacon's report. Fixed in `25c4433` by anchoring to _a_ loader report, which keeps the property the anchor exists for (only `entropy` sends an `R`, and a bare one would match `IRQs` or `RX`), and the stamp was taken again rather than claimed from a capture older than the corrected line |
| ADR-0091 (amended)  | Loader side tables under one hold             | The lock-order clause gained **SIDE → SCHED**, and the reason is a race that was live in the shipped product since [ADR-0088](adr/0088-product-home-cpu.md) pinned an agent to CPU 1: the spawn admits the task to the other CPU's queue, which could dispatch it before the loader recorded its manifest entry, and the agent then returned from `agent_body` without ever entering EL0. Found by `make check` on a busy host, not by review — `loader: a task reached the agent body with no manifest entry`, between beacon's line and chirp's. Measured under identical load: **3 boots in 8** lost an agent with the old ordering, **0 in 8** with the hold extended across the spawn. Gate: `no manifest entry` is now a product-boot negative **Pi stamp 2026-08-12, same transcript** — `beacon loaded … home=0` and `chirp loaded … home=1` on real dual-core silicon, both `ran … refusals=0`, and no `no manifest entry`: the record now wins against a CPU 1 that is genuinely another core rather than an emulated one                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| ADR-0028            | K1 EL1 wait-on-IRQ                            | `irq-wait: woke drops=0` (QEMU; Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ADR-0033            | K10 supervisor reap/restart                   | `supervisor: reaped id=…`, `supervisor: restarted id=…`, `supervised: cancelled` (QEMU; Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ADR-0034            | K9 RNG map agent                              | `rng-agent: map (read\|fault) ok`, `rng-agent: killed ok` (QEMU; Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ADR-0035            | P5 name registry                              | `name: resolved` / `name: missing`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ADR-0036            | P2 keyed blob store                           | `store: got` / `store: missing`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ADR-0037            | K3 EL1 cap transfer                           | `ipc: transfer ok`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ADR-0038            | K10 creator-exit cascade                      | `cascade: cancelled` (QEMU; Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ADR-0039            | P5 EL0 resolve                                | `el0-resolve: ok` / `el0-resolve: refused`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ADR-0040            | K2 EL1 park timeout                           | `ipc: timed-out cancelled`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ADR-0041            | K3 EL0 transfer self/creator                  | `el0-xfer: ok` / `el0-xfer: refused` (QEMU; Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ADR-0042            | K2 EL0 recv timeout                           | `el0-timeout: cancelled` (QEMU; Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ADR-0043            | K9 IRQ-cap device agent                       | `irq-device: woke wait_irqs=1` (QEMU; Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ADR-0044            | K5 thin stacks                                | `density: thin n=…` (QEMU; Pi stamp 2026-08-08; oracle census now `n=2` alongside Mini ADR-0086)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ADR-0045            | P2 durable region                             | `durable: reloaded` (QEMU; Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ADR-0046            | K4 cooperative budget                         | `budget: rotated` (QEMU; Pi stamp 2026-08-08 — historical). Oracle **superseded by ADR-0068** (`preempt-el1: rotated`): the IRQ epilogue wins the workers' voluntary check by construction. The quantum arithmetic lives on under both preemption paths                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ADR-0047 + ADR-0050 | K7 ASID first slice                           | `asid: dual a=… b=… ok`, with `asid: LEAK` / `asid: dual FAILED` asserted absent. QEMU + **Pi stamp 2026-08-09** (`20260809-100645.log` — earned the hard way: the first silicon boot was red, see the hardware-evidence section). Residuals split by [ADR-0084](adr/0084-k7-residual-policy.md): K7-M measure (optional lab), K7-T TTBR1 (trigger-gated), K7-R rollover                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ADR-0084            | K7 residual policy                            | **Design/policy only** — no status flip. Splits K7-M (switch-cost lab), K7-T (TTBR1 deferred with triggers), K7-R (ASID rollover); option C remains product regime. Optional later measure code; TTBR1 only under named trigger                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ADR-0085            | K5 density residual policy                    | **Design/policy only** — splits **K5-S** / **K5-H** / **K5-B**. Forbids `MAX_TASKS++` as density win. Code: [ADR-0086](adr/0086-k5-mini-stack-first-slice.md)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ADR-0086            | K5-S Mini stack first slice                   | `density: mini n=2 bytes_each=4096` (+ thin). Mini = one page, no unmapped guard. QEMU + **Pi stamp 2026-08-10**, transcript `20260810-162926-boot2-k5s.log` (`cpu: Cortex-A72`, `hw-transcript-check` clean)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ADR-0051            | K4 IRQ preemption design                      | Design accepted; fully implemented: re-audit ran for EL0 ([ADR-0064](adr/0064-k4-el0-preemption-first-slice.md)) and again for same-EL ([ADR-0068](adr/0068-k4-el1-preemption-second-slice.md), both **done (HW)**). No open same-EL re-audit; design ADR-0051 is complete on silicon                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ADR-0064            | K4 EL0 preemption first slice                 | `preempt: rotated` + `preempt: spinner exited irqs=4` (QEMU + **Pi stamp 2026-08-09**, transcript `20260809-122251.log`: PowerOn reset, `cpu: Cortex-A72 r0p3`, `CNTFRQ=54000000`). Host: `kernel_core::preempt` + `Switch::Preempt` model tests                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ADR-0052            | P5 resolve-grant                              | `resolve-grant: refused` + `el0-resolve: ok` under a granted task (QEMU + Pi stamp 2026-08-09)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ADR-0054            | K3 peer transfer                              | `el0-xfer-peer: ok` / `refused` / `donor emptied` (QEMU + Pi stamp 2026-08-09)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ADR-0055            | Transfer band filter                          | `xfer-peer: band refused` — a live task-cap moved as the object refuses (QEMU + Pi stamp 2026-08-09)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ADR-0056            | IPC ABI capacities                            | `make doc-claims` compares `src/ipc/mod.rs` constants to the ADR's table                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ADR-0057            | Task-cap lifecycle                            | `xfer-peer: stale refused` end-to-end; `mint FAILED`, `STALE MOVED` asserted absent; generation-wrap bound host-tested (the `STALE-TASKCAP` cross-check was deleted by ADR-0062: the epoch in the task identity makes its state unrepresentable)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ADR-0059            | Typed cap classification                      | quadrant + payload host tests in `kernel_core::cap`; band-refusal tests and `xfer-peer: band refused` unchanged                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ADR-0060            | Syscall reply layer                           | per-outcome host tests in `kernel_core::reply`; boot oracle exact counts unchanged (the byte-for-byte witness)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ADR-0061            | Refusal detail in x1                          | per-variant host tests incl. the stable-code table; `SessionStats::last_refusal_detail` carries it to the oracle — QEMU `el0-xfer-peer: refused … detail=4` (the discriminating assertion F-8 lacked)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ADR-0066            | P2 SD media durable store                     | Host: `kernel_core::{sdhci, sdcard, durable_media, mbr}` unit tests. QEMU: two boots on one scratch card image plus the no-card honest line. **Pi stamp 2026-08-09** (power-unplug protocol below: transcripts `20260809-140657.log` + `20260809-140804.log`, host `durable-read.sh` agreement)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ADR-0068            | K4 EL1 preemption second slice                | `preempt-el1: rotated` + `preempt-el1: spinner exited` — a non-yielding EL1 spinner loses the CPU on the vector epilogue (QEMU + **Pi stamp 2026-08-09**, transcript `20260809-151021.log`: PowerOn, `cpu: Cortex-A72 r0p3`, `CNTFRQ=54000000`, `hw-transcript-check` clean). Predicate reused from ADR-0064; pivot asm is the ADR's review artifact                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ADR-0048            | K8 SMP design                                 | Design accepted; implementation arc through steal paid **done (HW)** (ADR-0070…0083). Residual: agent+TLB steal only if product auto-balances EL0 agents                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ADR-0070            | K8 first slice (unpark core 1)                | `smp: core1 alive` — QEMU `raspi4b -smp 4` (spin-table PA `0xe0` + in-kernel table + PoC-clean root handoff). **Pi stamp 2026-08-09**, transcript `20260809-160348.log` (`cpu: Cortex-A72`, `CNTFRQ=54000000`, `hw-transcript-check` clean). Later queues/steal: ADR-0076…0083                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ADR-0074            | K8 second slice (SGI IPI wake)                | `smp: core1 ipi` — QEMU + **Pi stamp 2026-08-10**, transcript `20260810-130305.log` (`cpu: Cortex-A72 r0p3`, `CNTFRQ=54000000`, `hw-transcript-check` clean). Handler seal count is 3 (timer + UART + wake SGI)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ADR-0075            | K8 per-core queues design                     | Design accepted; first code [ADR-0076](adr/0076-k8-per-core-queues-first-slice.md)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ADR-0076            | K8 per-core queues first slice                | `smp: core1 ran` — dual-current pure model, `spawn_on(1)`, SGI resched, marker runs and exits. QEMU + **Pi stamp 2026-08-10**, transcript `20260810-130305.log` (`cpu: Cortex-A72`, `CNTFRQ=54000000`, `hw-transcript-check` clean; same boot as IPI/alive)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ADR-0079            | K8 per-core timer + EL1 preempt on CPU 1      | `preempt-el1-cpu1: rotated` + `preempt-el1-cpu1: spinner exited` — local CNTP on affinity 1, global ticks CPU0-only, EL1 epilogue fence lifted (design ADR-0078). QEMU + **Pi stamp 2026-08-10**, transcript `20260810-132749.log` (`cpu: Cortex-A72 r0p3`, `CNTFRQ=54000000`, `hw-transcript-check` clean; build `src=385cccee`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ADR-0077            | SMP shared-state discipline                   | Heap + sched + park under the kernel mutex; single `arch::smp` resched; per-CPU mirrors. **F-R1-P1 complete (HW):** IPC, frames, ASID, naming, storage, taskcap, durable, console TX/RX model, IRQ wait/caps, MMU map/unmap (stamp 2026-08-10, `20260810-160227.log`). **Amended 2026-08-11:** loader `ENTRY_OF_TASK`/`ACTIVE` locked (missed after product `home_cpu` dual-core); then the whole discipline restated as `sync::Mutex<T>` by [ADR-0091](adr/0091-data-in-lock.md) — same five steps, datum inside the lock. Residual: lock refinement if measured; agent+TLB steal; coarse SCHED lock is N=2 only. **Pi stamp 2026-08-11, transcript `20260811-122821.log`** — one oracle boot on Pi 4B silicon (`cpu: Cortex-A72 r0p3`, `src=1ed04fbe`, `boot=21 from=Previous`), `make hw-check` clean: every shared table, the scheduler switch path and the MMU arena ran under `Mutex<T>` on hardware (ADR-0091)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| ADR-0092            | Supervisor lifecycle verdicts                 | `kernel_core::lifecycle` decides reap / force / cancel from `is_idle` + `Option<State>`; the full 10-input table host-tested, including the deliberate `Empty` divergence (reap `NotBlocked` vs force `Empty`). In the mutation scope from its first commit. `sched` keeps lock, TCB flag, counters, SGI. Behaviour unmoved: reap / cascade / force-exit oracle lines unchanged. **Pi stamp 2026-08-11, transcript `20260811-122821.log`** — one oracle boot on Pi 4B silicon (`cpu: Cortex-A72 r0p3`, `src=1ed04fbe`, `boot=21 from=Previous`), `make hw-check` clean: `supervisor: reaped` and the force-kill sequence took their verdicts from `lifecycle` on silicon                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ADR-0093            | Panic path positive evidence                  | `make panic-check` — a `panic-probe` image writes to a real task stack's guard page after announcing its address. Twelve assertions: the probe ran and said where, the handler was reached from a _trap_ (`sync exception EL1`), the printed `FAR` **equals the announced address**, the guard is named as a stack overflow, `KERNEL PANIC` appears once (re-entry guard held), `Harbor: hello` appears once (`cpu::halt` stopped the core), nothing follows `*** halt ***`. Seen red twice: announcing a different address than it writes fails on the `FAR` comparison; removing the store fails with "announced, never panicked". **Pi stamp 2026-08-11, transcript `20260811-122821.log`** — the fault on silicon: `panic-probe: stack guard at 0x129000, writing` → `*** KERNEL PANIC ***` → `sync exception EL1: data abort … translation fault level 3 on write` → `FAR=0x…129000` → the stack-overflow line → `*** halt ***`. This is the one that earns most from hardware: TLB fills are speculative on Cortex-A72 and not in TCG, so _the guard page faults_ is a claim only the board settles                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ADR-0094            | Retire `debug-display`                        | The absence is the evidence: no `debug-display` in `Cargo.toml`, the `Makefile` or `src/`; `make check` green with one target fewer; the boot oracle now refuses **any** `display:` line, so a driver returning without a composition fails a gate rather than passing quietly. `kernel_core::{display, textgrid, font8x8, spi}` keep their host tests and their place in the mutation scope. **Pi stamp 2026-08-11, transcript `20260811-122821.log`** — one oracle boot on Pi 4B silicon (`cpu: Cortex-A72 r0p3`, `src=1ed04fbe`, `boot=21 from=Previous`), `make hw-check` clean: the board reports `discover: display compiled=off` and `build: headless (no bring-up gates)`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ADR-0095            | Boot phases have names                        | `bootstrap::run` is a list of named calls; each phase carries the prose that used to sit above its block. No behavioural change, so the evidence is the gates staying green across the extraction: `boot-check`, `product-boot-check` and `panic-check` after every commit. The one `unsafe` involved (`console::enable_rx_irq`) had its SAFETY re-derived rather than moved, because it named the function it lived in. **Pi stamp 2026-08-11, transcript `20260811-122821.log`** — one oracle boot on Pi 4B silicon (`cpu: Cortex-A72 r0p3`, `src=1ed04fbe`, `boot=21 from=Previous`), `make hw-check` clean: the phase sequence is the boot the board printed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| ADR-0096            | Gates that do not depend on remembering       | Three rules that were enforced by habit now fail: `make mutation-freshness` (seen red at 608 vs a stamp of 607), `make hw-check` (the hardware gate had no target and was typed from memory; it now also names the capture's `src=` against the tree), and `ALLOW_*_SKIP` refused when `CI=true` (seen red). The failure that prompted it: fourteen K8 survivors found by the first run in twenty-odd commits                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ADR-0097            | Loader plan as data                           | `kernel_core::loaderplan` decides source, empty-table refusal, and per entry `validate` → `bind` → act; nine host tests including the one that pins the order (an entry both malformed and over-reaching is reported as malformed). In the mutation scope from its first commit. `loader:` oracle lines unchanged in `boot-check` and `product-boot-check`. **Pi stamp 2026-08-11, transcript `20260811-122821.log`** — one oracle boot on Pi 4B silicon (`cpu: Cortex-A72 r0p3`, `src=1ed04fbe`, `boot=21 from=Previous`), `make hw-check` clean: `loader: builtin` → `beacon loaded` / `mute loaded`, planned then executed on hardware                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ADR-0078            | K8 per-core timer + preempt design            | **Design only** — code [ADR-0079](adr/0079-k8-per-core-timer-preemption-first-slice.md) **done (HW)**. CPU0 owns global `ticks()`; each CPU programs CNTP + banked PPI 30; EL1 epilogue on CPU1; EL0-on-CPU1 deferred to [ADR-0080](adr/0080-k8-el0-on-cpu1-design.md)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ADR-0080            | K8 EL0-on-CPU1 design                         | **Design only** — code [ADR-0081](adr/0081-k8-el0-on-cpu1-first-slice.md) **done (HW)** (stamp 2026-08-10, see its row). Per-CPU `CURRENT_EL0`; publish on every affinity; sticky home; EL0 preempt on CPU1; steal still later                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ADR-0081            | K8 EL0-on-CPU1 first slice                    | `preempt-el0-cpu1: rotated` + `preempt-el0-cpu1: spinner exited` — per-CPU publish, EL0 quantum on home=1 (design ADR-0080). QEMU + **Pi stamp 2026-08-10**, transcript `20260810-134826.log` (`cpu: Cortex-A72 r0p3`, `CNTFRQ=54000000`, `hw-transcript-check` clean; build `src=b898ebcd`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ADR-0082            | K8 work-stealing design                       | **Design only** — code [ADR-0083](adr/0083-k8-work-stealing-first-slice.md) **done (HW)** (stamp 2026-08-10, see its row). Hard re-home; pull-on-idle; opt-in stealeable (no agent AS without TLB IPI)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ADR-0083            | K8 work-stealing first slice                  | `smp: steal ok` — victim admitted on CPU0 only later runs on affinity 1 (design ADR-0082). QEMU + **Pi stamp 2026-08-10**, transcript `20260810-144305.log` (`cpu: Cortex-A72 r0p3`, `CNTFRQ=54000000`, `hw-transcript-check` clean; build `src=5c7c4d2c`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ADR-0071            | H3 L0 x86_64 QEMU first slice                 | `make x86-boot-check` — `Harbor: hello (x86 lab)`, `cpu: … family=… model=…`, `x86-lab: alive` under `qemu-system-x86_64 -machine q35 -kernel harbor-x86.elf` (PVH note). Status **done (QEMU-x86)** only; never collapsed into AArch64 `done (QEMU)` or `done (HW)`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ADR-0072 + ADR-0073 | Hardware self-discovery (FDT report)          | `discover: model` / `memory` / `cpus` / `display` lines — host tests on fixture DTB; QEMU first boot with `-dtb` fixture, second boot DTB-less `unknown (no dtb)`; verify-don't-select (no map consumption). **Pi stamp 2026-08-10**, transcript `20260810-030801-boot2.log` (`model … Rev 1.5 rev=0xc03115`, `memory 3956 MiB (2 ranges) beyond compiled map`, `cpus 4 … matches`, `hw-transcript-check` clean)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |

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

The rule earned its second example on 2026-08-09. TCG never fills the TLB
speculatively; a Cortex-A72 does. When ADR-0050 removed the per-switch
`tlbi vmalle1is`, nothing retired the early map's Global 1 GiB blocks — QEMU
stayed green for a day, and the first silicon boot served the first EL0
fetches from a stale EL1-only block: instruction abort, permission fault
level 1 (`.serial-log/20260809-093312.log`, three oracle lines red). The fix
is `mmu::activate`'s one-time `retire_early_map`, the ADR-0050 amendment —
and the gate that can actually see this class is
`scripts/check/hw-transcript-check.sh`, the same oracle assertions
(`scripts/lib/boot-oracle.sh`, one owner) run against a hardware transcript.

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
| `kernel_core::syscall::decode` (+ `SYS_PUTC`) | **closed**             | host unit tests (`make doc-claims` owns the current suite count)                                                                           |
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
[architecture.md § Completeness roadmap](architecture.md#completeness-roadmap).

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

Deploy:

```bash
# Product (no oracle, store injected) — what `make deploy` flashes.
make deploy SD_MOUNT=/run/media/$USER/bootfs
# Lab/oracle fleet (hw-check against boot-oracle.sh):
make deploy-oracle SD_MOUNT=/run/media/$USER/bootfs
```

## Hardware evidence: ADR-0066 power-cycle stamp (2026-08-09)

The claim is cross-power-cycle persistence, so the stamp is **two** powered
boots and an independent read, not one transcript:

1. `scripts/host/durable-partition.sh /dev/sdX` once (card in the reader),
   then `make deploy` — the deploy guard reports the partition.
2. Boot 1, captured: `reset: PowerOn`, `durable-media: boot=1 from=Fresh`,
   `flushed seq=1`, `verified`.
3. **Physically unplug power** for ≥5 s — not a watchdog, not a reboot;
   `reset: PowerOn` on silicon is what distinguishes it.
4. Boot 2, captured: `durable-media: boot=2 from=Previous seq=1`. This is
   the canonical transcript: `hw-transcript-check` requires
   `from=Previous boot>=2`, so one log carries the cross-cycle evidence.
5. Card back in the reader: `scripts/host/durable-read.sh /dev/sdX` — dd +
   CRC with nothing of the kernel in the loop must agree with boot 2's
   claims.

**Recorded (2026-08-09, Pi 4B, card `/dev/sdb3` = MBR type 0x7f at LBA
2048):**

- Boot 1 — `.serial-log/20260809-140657.log`: `reset: PowerOn`,
  `durable-media: boot=1 from=Fresh part=0x7f slot=- seq=0 host=emmc2`,
  `flushed slot=A seq=1`, `verified`. Silicon answered on **EMMC2** (QEMU
  answers on the Arasan — the dual-probe bind reports both honestly).
- Physical power unplug ≥5 s.
- Boot 2 (canonical) — `.serial-log/20260809-140804.log`:
  `reset: PowerOn`, `durable-media: boot=2 from=Previous part=0x7f slot=A
seq=1 host=emmc2`, `flushed slot=B seq=2`, `verified` —
  `hw-transcript-check` clean.
- Host reader — `durable-read.sh /dev/sdb`: `slot=A seq=1 … crc=ok
durb=ok`, `slot=B seq=2 … crc=ok durb=ok`. dd + CRC with no kernel in
  the loop agrees with both boots' claims.

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

## Hardware evidence: H1 depth stamps on silicon (2026-08-08)

Pi 4B, PL011 via CP2104-class USB-TTL @ 115200, `.serial-log/20260808-030219.log`,
image `kernel8.img` 141108 B from commit `a7ff8d8` (headless). Deployed with
`make deploy SD_MOUNT=/run/media/gianluca/bootfs`.

Silicon markers (not TCG): `CNTFRQ=54000000 Hz`, `rng200: ok word=0x1c11a56a`.

One cold boot exercised the H1 entry oracles **and** the 2026-08-08 depth
slices that had been QEMU-only:

| Claim                       | Serial evidence                                                                      |
| --------------------------- | ------------------------------------------------------------------------------------ |
| K5 thin stacks              | `density: thin n=3 bytes_each=8192` (historical census; today `n=2` + Mini ADR-0086) |
| P2 durable section          | `durable: reloaded`                                                                  |
| K4 cooperative budget       | `budget: rotated`                                                                    |
| K9 IRQ-cap device wait      | `irq-device: woke wait_irqs=1` (with `el0-irq: woke`)                                |
| K1 EL1 wait-on-IRQ          | `irq-wait: woke drops=0`                                                             |
| K10 supervisor reap/restart | `supervisor: reaped id=…` / `supervisor: restarted id=…`                             |
| K9 RNG map agent            | `rng-agent: map read ok` / `rng-agent: killed ok`                                    |
| K3 EL0 transfer             | `el0-xfer: ok` / `el0-xfer: refused`                                                 |
| K2 EL0 recv timeout         | `el0-timeout: cancelled`                                                             |
| K2 EL1 park timeout         | `ipc: timed-out cancelled`                                                           |
| P5 names / P2 RAM store     | `name: resolved` / `store: got`                                                      |
| K3 revoke                   | `ipc: release stale refused`                                                         |
| K10 cascade / auto-reap     | `cascade: cancelled` / `ipc: auto-reaped cancelled`                                  |
| Steady timer                | `ticks=10` … (CNTFRQ path)                                                           |

Representative lines (host timestamps from `serial-capture`):

```
03:03:01.251539 rng200: ok word=0x1c11a56a
03:03:01.251803 CNTFRQ=54000000 Hz  timer=10 Hz  PPI=30
03:03:01.316487 density: thin n=3 bytes_each=8192
03:03:01.316593 durable: reloaded
03:03:02.137115 irq-device: woke wait_irqs=1
03:03:02.137201 budget: rotated
03:03:02.214337 ticks=10
```

**Blind spots of this stamp:** no SPI TFT (`FEATURES` none); PL011 RX-own
handshake incomplete on this boot (`rx own short` / incomplete — not a reopen
of M6 if prior stamps hold); true SD power-cycle of the durable section not
re-read after power loss; peer transfer / resolve-grant landed **after** this
stamp and were QEMU-only at the time (ADR-0052/0054) — closed by the
2026-08-09 stamp below.

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

| Absence                        | Count | What its presence would have meant                                                      |
| ------------------------------ | ----- | --------------------------------------------------------------------------------------- |
| `panic`                        | 0     | Any assertion fired, including `el0: published session is not the current task's`       |
| `no published session`         | 0     | The vector path read a stale pointer across the park's switch — the one ADR-0019 guards |
| `Xel0`                         | 0     | The console-less agent's byte reached the UART                                          |
| `loader: … FAILED` / `refused` | 0     | An entry the manifest declared could not be bound or created                            |

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
performed one at the time of this record. A preemptive scheduler is what
would — and since [ADR-0064](adr/0064-k4-el0-preemption-first-slice.md) the
EL0 preemption slice performs exactly that rotation (`preempt: rotated`),
resting on the same per-task session state this section argued for. Same-EL
preemption is closed by [ADR-0068](adr/0068-k4-el1-preemption-second-slice.md)
(**done (HW)**, Pi stamp 2026-08-09, transcript `20260809-151021.log`). Residual
under **K4**/sched policy is priorities and quantum policy, plus multi-core IPI
preemption under **K8** — not same-EL itself
([ADR-0026](adr/0026-kernel-and-product-completeness.md)).

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

The banner is the product of the feature split (rule 9): an image says what it
is on the wire, before a missing probe has to be diagnosed from absence. This
is the first hardware boot where the kernel declares its own build. It read
`oracle`/`bringup`/`debug-display` at the time; the panel half went with
[ADR-0094](adr/0094-retire-debug-display.md) and `panic-probe` took its place
in the split.

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
| `full=0 state=0`           | No mailbox filled and no endpoint resolved to a dead slot; the exact authority ledger is asserted separately by the boot oracle  |

The counters are asserted by producer in `scripts/lib/boot-oracle.sh`: the
current manifest and revoke demos deliberately produce seven IPC-send
authority refusals. Per-session reply refusals remain separate and do not enter
this machine-wide IPC counter.

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
early MMU active — **undated**: this predates the stamped-transcript convention
and carries no log path or commit, so treat it as historical until the next
bring-up re-stamp records one:

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

| Check                                                            | Mutation                                                                                                                               | Observed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| PL011 divisors, bump alignment, `TCR.EPD1`, descriptor alignment | original implementations                                                                                                               | 10 red tests before the fixes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| SPSC ring ordering                                               | publish `head` before writing the slot                                                                                                 | `out of sequence at 8572`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Allocator coalescing                                             | drop the backward merge                                                                                                                | `arena must be whole again`, `churn left the arena fragmented`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| L3 descriptor encoding                                           | encode an L3 leaf as a block                                                                                                           | `L3 leaf must be 0b11`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| No-SIMD guard                                                    | the pre-softfloat image                                                                                                                | `dup v0.4h` in `memset`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Pre-MMU path                                                     | a Rust `fetch_add` called from `_start`                                                                                                | named the symbol and explained the fix                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| QEMU boot check                                                  | remove `irq::enable(TIMER_IRQ)`                                                                                                        | missing tick reports                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Starved-host verdict (ADR-0087)                                  | `CPUQuota=25%`, `15%` and `10%` on the same clean image                                                                                | `INDETERMINATE` with the share it was decided on (0.22/0.13 starved, 0.08 not credible), exit 3 — and clean at 2.14                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| HW transcript slicing                                            | `hw-transcript-check.sh` on a capture holding two power cycles                                                                         | `task output not interleaved` with every line twice — the ADR-0077 stamp `20260810-160227.log`, cited here as clean, no longer was. The checker now asserts the **last** boot and prints which of how many                                                                                                                                                                                                                                                                                                                                                                                                         |
| Trap frame coupling                                              | grow `TrapFrame` by 16 bytes                                                                                                           | the stub's reservation moved `0x110` → `0x120`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Blob integrity                                                   | corrupt an expected hash                                                                                                               | refused to install, exit 1                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Miri                                                             | publish `head` before writing the slot                                                                                                 | `Undefined Behavior: Data race detected between (1) non-atomic write and (2) non-atomic read`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `mmu::map` overwrite refusal                                     | map the same region twice                                                                                                              | `AlreadyMapped(0x8000000)` instead of a silent replacement                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Bring-up build gate                                              | rename a function used only there                                                                                                      | `make bringup-builds` red, `E0425`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Layout validator                                                 | `GUARD_PAGE_SIZE = 0` in `link.ld`                                                                                                     | `LAYOUT INVALID: GuardIneffective` — and the first attempt at that check passed, which is how the linker-symbol fold below was found                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Refusal to boot unprotected                                      | make `mmu::activate` return `OutOfTables`                                                                                              | `BOOT REFUSED: could not map planted failure` and then nothing — no heap line, no ticks, no console loop                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Pre-MMU path, indirect branch                                    | reach the gate through `blr x9`                                                                                                        | `indirect branch in _start: its target is not derivable` — the call graph the check walks had a hole                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Layering rules                                                   | `drivers` imports `bsp`; `arch` imports `drivers`; `exception` imports `drivers`                                                       | one line naming the module and the edge, for each of the three rules separately                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| RX bytes dropped                                                 | shrink the ring to 4 bytes and paste 60                                                                                                | `console: DROPPED 57 received bytes (ring full)`, where before the loss was invisible                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Exception stack (`SP_EL1`)                                       | run the same overflow on the pre-split tree                                                                                            | `FAR=0x9c000`, the guard's **bottom**, against `0xa1ff8`, its **top** — the handler had walked the whole page and landed below it                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Exception-stack guard page                                       | zero-length exception guard in `Boundaries`                                                                                            | `GuardIneffective` — validation is written once over both stacks, and this is what keeps that true                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Double-free refusal (the mark)                                   | stop consulting the allocated bit                                                                                                      | one test red — the one where alignment leaves the back-pointer intact, which is the only case the sentinel cannot catch                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Double free through the real allocator                           | free the same pointer twice in `console_loop`                                                                                          | `heap: REFUSED 1 invalid frees`, boot check red, and the heap still `fully reclaimed`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Doc claims (test count)                                          | restore the stale `54 host unit tests`                                                                                                 | `README claims 54 host unit tests, there are 77` — the exact drift it was written for                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Doc claims (gate list)                                           | drop `bringup-builds` from the README                                                                                                  | printed both lists side by side; this is F27, which had already happened twice for real                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| TLBI operand shift                                               | drop the `>> 12`                                                                                                                       | three tests red — the operand became the address, invalidating a different page                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Runtime mapping (`mmu::map`)                                     | skip the call, keep the read                                                                                                           | `ESR=0x96000006` level-2 translation fault at the blob address; with the call, `0xd00dfeed`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Cooperative interleaving (M3)                                    | make `sched::yield_now` a no-op                                                                                                        | `task output not interleaved:` with an empty list — idle spun on `has_ready` and no worker ever ran                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Block split (M3)                                                 | aim the split smoke at an already-L3 page                                                                                              | `block split path did not run: split: page at 0xb5000 split 0, remapped` — the line is there, the split is not                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `Context` / assembly coupling (M3)                               | swap `x30` and `sp` in `Context`                                                                                                       | two `offset_of` asserts red at compile time, naming both offsets; the size assert alone stayed green at 104 bytes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Table-arena reserve (M3)                                         | raise `MIN_SPARE_TABLES` to 40                                                                                                         | `BOOT REFUSED: table arena nearly exhausted: 10 tables left, need 40 (raise PAGE_TABLE_ARENA_SIZE in link.ld)` and then nothing                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| SPI divisor overflow                                             | range-check after rounding instead of before                                                                                           | `left: Ok(0)` against `right: Err(TargetTooSlow …)` — a wrapped divider is a _legal_ encoding, so the fastest request became the slowest clock                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| MMIO probe window (`FAR` match)                                  | drop the `far != expected` check, and fault twice inside one probe                                                                     | without it both aborts are swallowed and the boot continues (`rng200: unavailable`); with it the second is fatal — `ESR=0x96000050 FAR=0xfe105000`, the injected address                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Table-arena reserve, derived                                     | restore `PAGE_TABLE_ARENA_SIZE = 16 * 0x1000` under the reserve now derived from `MAX_TASKS`                                           | `BOOT REFUSED: table arena nearly exhausted: 9 tables left, need 14` — the arena had been sized against a reserve of six that assumed `MAX_TASKS = 4`, long after the scheduler raised it to 12                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Facade isolation (ADR-0015)                                      | `use crate::arch::riscv64::cpu`, `use crate::{arch::aarch64::cpu, bsp}`, `use crate::bsp::rpi4::memmap` in one file outside both trees | three violations named with their line numbers — the first two were invisible to the gate as first written, which listed `aarch64` literally and looked for the `crate::` prefix a grouped import does not carry                                                                                                                                                                                                                                                                                                                                                                                                   |
| Arch contract vs facade                                          | delete the `probe` row from `arch-contract.md`                                                                                         | `missing from the contract: probe` — the surface a port is written against and the surface the facade actually re-exports had nothing comparing them                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| IRQ dispatch table seal                                          | register a handler _after_ `irq::seal()`, on the real kernel path                                                                      | `MUTATION: post-seal register -> Err(Sealed { irq: 7 })`, and `irq: sealed with 2 handlers registered` unchanged. The seal is what makes the IRQ path's shared `&'static` borrow sound, and until `kernel_core::irqtable` existed nothing had ever registered after sealing to watch it refuse — the invariant the safety argument rests on was asserted by a comment                                                                                                                                                                                                                                              |
| Dispatch table populated                                         | drop the `println!` reporting the seal count                                                                                           | `boot-check: FAIL — dispatch table sealed with the wrong number of handlers: (no seal line at all)`. A boot that registers nothing is indistinguishable from a healthy one until the first interrupt nobody answers                                                                                                                                                                                                                                                                                                                                                                                                |
| ADR table in `architecture.md`                                   | run the new `xrefs` check against the table as it stood                                                                                | `0015-multi-arch-scaffold.md is missing from the artefact table`, and the same for 0016. Both had been written, accepted and merged while the table a reader meets first still stopped at ADR-0014 — the third copy of a fact the gate was already comparing in two places                                                                                                                                                                                                                                                                                                                                         |
| `CURRENT_EL0` published on switch                                | delete `publish_el0(sched, to)` from `switch_with`, boot                                                                               | `panicked at src/arch/aarch64/el0.rs: el0: published session is not the current task's (stale after switch)` on the first EL0 entry from a spawned task. Before slice 1 this row read "Nothing yet" in ADR-0017 — a stale pointer is silent until one agent reads another's saved registers, so the check shipped in the same commit as the pointer                                                                                                                                                                                                                                                                |
| No-`static mut` (ADR-0019)                                       | restore `static mut CURRENT_EL0: *mut El0Session`                                                                                      | `no-static-mut: src/arch/aarch64/el0.rs:…: static mut CURRENT_EL0:…` then exit 1 — the gate greps declarations, not prose, so comments that name the form stay green                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| IRQ scope (ADR-0022)                                             | put `sched::yield_now()` inside `resume_step`'s `without_irqs`                                                                         | `irq-scope: src/agent/mod.rs:178: \`yield_now\` is inside the \`without_irqs\` opened at line 177` then exit 1 — the region is found by brace depth, so the call does not have to be on the opener's line                                                                                                                                                                                                                                                                                                                                                                                                          |
| Syscall ABI in the threat model (ADR-0017/0022)                  | delete the `SYS_PUTC` row from `SECURITY.md`'s authority table                                                                         | `doc-claims: the syscall ABI and SECURITY.md's authority table disagree` naming `SYS_PUTC(2)`, then exit 1 — the set is compared both ways, so an invented row fails too                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `El0Session` field offsets                                       | insert a field before `user_ttbr`                                                                                                      | eight `offset_of` assertions red at compile time, each naming its field and its expected offset. The assembly does not actually drift — its offsets are `.equ` symbols derived from the same struct — so this is a tripwire on an _unintended_ reorder rather than the mechanism keeping the two in agreement                                                                                                                                                                                                                                                                                                      |
| Stale `#[allow(dead_code)]`                                      | convert all thirteen to `#[expect(…, reason = …)]`                                                                                     | three came back _unfulfilled_: `TrapFrame`, `frames::alloc` and `frames::free` have had consumers for milestones while an attribute went on calling them dead. `allow` is silent forever; `expect` warns the moment the deroga stops being needed, which is the only difference and the whole reason to prefer it                                                                                                                                                                                                                                                                                                  |
| Scaffolding in the product image                                 | pull `demos` back in through an inner `mod` with `#[path]`, so it compiles without the feature                                         | **Twice green before it was right.** v1 grepped `llvm-nm` for `bootstrap::demos` and reported clean with 4 KiB of demo code in the image — release LTO renames and inlines, so the module path is not in the symbol table. v2 listed six console markers by hand and passed the same leak, because the leaked function's output was not among the six. v3 derives every literal from `demos.rs`, validates each against the image that _has_ the oracle, and catches it: `'el0: SVC ok  imm=0' is in an image built without the oracle`                                                                            |
| EL0 program encoding                                             | change the `tbnz` offset in `encode_pl011_rx_poll_exit` from 4 to 3                                                                    | `tbnz w1, #4, #12` against the intended `#16` — the branch target, in the disassembly, beside the assembly it is meant to be. Without the test the same mistake produces `rx poll unexpected putcs=…` on a board and reads like a kernel bug                                                                                                                                                                                                                                                                                                                                                                       |
| The assembler is missing                                         | shadow `llvm-mc` with a command that exits non-zero                                                                                    | the test **fails** rather than skipping. `make no-simd` once reported `clean` having disassembled nothing, and that lesson is in this helper's doc-comment                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Coupling a test to a tool's _output_ format                      | push the first version, which disassembled and compared mnemonics                                                                      | **CI, on the first push**: the runner's `llvm-mc` prints a `.text` directive the development machine's does not. Local green, remote red, and the fix was not to filter the directive — it was to invert the direction. Disassembly output is a rendering; assembly input is a language. The intended text now goes through the assembler and the comparison is on bytes                                                                                                                                                                                                                                           |
| Doc symbol paths                                                 | put `arch::mmu::EARLY_L1` back into `docs/mmu.md`                                                                                      | `doc-symbols: EARLY_L1 lives in src/mm/early.rs, which is not a module 'mmu'`. This is the sentence F23 left behind for a day: the finding was that board topology does not belong in `arch`, and the document explaining the map still put it there. Asking only whether `EARLY_L1` exists would have passed                                                                                                                                                                                                                                                                                                      |
| Scheduler model, idle requeue                                    | make `Switch::Yield` requeue everything _except_ idle                                                                                  | `invariant broken after step 2: idle is not current and nothing is queued` with the counter-example `[Admit, Switch(Yield)]`. Two operations, and nobody would have written that test — the first version of the invariant asserted `state(IDLE) == Ready`, which the mutation satisfies while idle sits outside the queue. The model found the _specification_ too weak before it found anything about the code                                                                                                                                                                                                   |
| IPC model, generation check                                      | drop `ep.generation != cap.generation()` from `Table::lookup`                                                                          | `diverge at step 2 — Send(Stale): reference says Err(BadCap), table says Ok(None)`, counter-example `[Create, Send(Stale)]`. This is the check `SECURITY.md` calls latent: no kernel path mints a stale handle, so nothing exercised it until the model offered one at every step                                                                                                                                                                                                                                                                                                                                  |
| IPC model, full mailbox                                          | `mbox.len == DEPTH` → `mbox.len > DEPTH`                                                                                               | `diverge at step 4 — reference says Err(Full), table says Ok(None)` with `[Create, Send, Send, Send]`. The off-by-one that lets a bounded queue grow by one                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| Image declares a feature set it does not have                    | make the headless banner claim `debug-display`                                                                                         | `boot-check: FAIL — image says debug-display, but the panel never came up`. Checked in both directions: an image claiming the panel must bring it up, one claiming headless must not touch it. Neither half alone is enough — each is satisfiable by a lie. **Since [ADR-0094](adr/0094-retire-debug-display.md)** there is no panel to claim, so the surviving half refuses _any_ `display:` line: the failure to watch for is a driver returning without a composition                                                                                                                                           |
| Console denied by default (ADR-0017 §3)                          | grant `CONSOLE_SLOT` to the agent that is meant to lack it                                                                             | the refusal line disappears and the byte `X` appears on the console. Both halves are asserted: the boot check fails if the denial line is missing _and_ if the denied agent's byte shows up                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `SessionEnd` swallowed (ADR-0018 §4)                             | read `s.end` and drop it                                                                                                               | `error: unused agent::SessionEnd that must be used`, carrying its own note — _the creator decides what happens to a faulting agent; the kernel only ended its session_. Under `-D warnings` that is a build failure, which is the whole point                                                                                                                                                                                                                                                                                                                                                                      |
| Creator survives its agent's fault                               | remove the `creator alive after fault` line                                                                                            | `boot-check: FAIL — the creator did not survive its agent's fault`. One line saying "it faulted" would have hidden the two claims that matter: the creator kept running, and so did its peer                                                                                                                                                                                                                                                                                                                                                                                                                       |
| Orphaned trait behind a feature (retired)                        | remove the attribute from `SpiDevice` and build `--features debug-display`                                                             | `trait SpiDevice is never used`. **No longer runnable** — the trait and the feature went with [ADR-0094](adr/0094-retire-debug-display.md), which is the retirement this row's own argument was pointing at. It has an implementation (`ExclusiveDevice`) and no caller in any configuration. ADR-0010's _requirement_ — must not bit-bang CS — is satisfied by `with_bus`; only a sentence beside it, saying short ops use `SpiDevice::write`, stopped describing anything. [ADR-0020](adr/0020-spidevice-contract-without-a-caller.md) retracts the sentence and keeps the trait as the contract ADR-0009 adopts |
| Slot bound (`cap::from_slot`)                                    | `slot >= caps.len()` → `slot > caps.len()`                                                                                             | two host tests red, both by index-out-of-bounds: the last-slot test and the empty-table test. The bound is the whole of slot-indexed authority — one past it is an agent reading a word of someone else's table                                                                                                                                                                                                                                                                                                                                                                                                    |
| EL0 authority refusal                                            | run the boot with the agent that names slot 1 removed                                                                                  | `boot-check: FAIL — EL0 agent was not refused a slot it does not hold`. The refusal is on the _good_ path on purpose: a protection nobody watches fire is an assumption                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| Payload crosses EL0 → EL0                                        | drop the `mov x0, x2` from the receiving agent, so it prints its status instead of the message                                         | `boot-check: FAIL — the received payload was not printed by the receiving agent`. Without the move the agent prints a zero and proves only that it resumed                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| Boot check vs a binary log                                       | the same mutation, before `-a` was added to every grep                                                                                 | `FAIL — task output not interleaved`, naming the wrong assertion entirely: the agent's zero byte made `grep` treat the log as binary and stop matching. An agent can now `SYS_PUTC` any byte, so this stopped being hypothetical                                                                                                                                                                                                                                                                                                                                                                                   |
| Authority counter vs a full mailbox                              | five EL0 sends into a four-deep mailbox                                                                                                | `refuse count=2 full=1` — the fifth send is `full`, and the authority count does not move. This is what the counters are separate for                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| Authority count survives later traffic                           | host test: note a refusal, then a successful send + recv                                                                               | the count stayed at 1 rather than being erased. It _was_ erased before this slice — see below                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ADR status, three copies                                         | stamp `accepted` in ADR-0017's frontmatter alone, then run `xrefs`                                                                     | both other copies named in one run: `status is 'accepted', the index says 'proposed'` and `architecture.md does not mark it (**accepted**)`. Accepting an ADR means moving three files, which is exactly the shape that goes stale by attention                                                                                                                                                                                                                                                                                                                                                                    |
| README module map                                                | run the new `doc-claims` check against the Layout block as it stood                                                                    | twenty of `kernel-core`'s twenty-five modules named as missing, plus `src/agent` — the agent shell, this project's central concept, absent from its own map — and `time.rs` listed as `time/`                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Board addresses inside the ISA tree                              | `const PERIPHERALS: u64 = 0xC000_0000;` and `RAM_TOP = 0x8000_0000` put back into `arch/aarch64/mmu.rs`                                | `arch-board-free: … names 0xC000_0000, a physical range base`, both lines, exit 1. This is F23, which stayed open for two days with `make layering` one directory away — that gate sees imports, and the other way to know a board is to write its addresses out by hand                                                                                                                                                                                                                                                                                                                                           |
| UART RX handover order                                           | swap the two steps in `RxLine::plan_suspend`, then in `plan_resume`                                                                    | five tests red for the first, two for the second, each naming the exact step: `step 0 (ClearView) left the line armed with no view`. This is the defect a review found by reading — the window is an instruction pair wide and the boot check types nothing — and until `kernel_core::rxline` existed the only evidence it was fixed was a hardware boot nobody re-runs                                                                                                                                                                                                                                            |
| Boot check, host starvation                                      | `systemd-run --user --scope -p CPUQuota=8%` around the boot check, the level at which `timer: MISSED` first appears                    | `boot-check: INDETERMINATE — … the emulator got 0.07 cores of host CPU over 15s`, exit 3. The same script on an idle host reports `2.97 cores` and passes; with the assertion rewired to a line that is always present it reports `FAIL — … the emulator had the CPU to meet them`. All three outcomes seen, which is what makes the third one a verdict rather than a comment                                                                                                                                                                                                                                     |
| No-SIMD guard, tool absent                                       | `make no-simd OBJDUMP=llvm-objdump-does-not-exist`                                                                                     | `no-simd: FAIL — refusing to report clean`. Before the check, the same run printed `no-simd: clean`: an empty pipeline made `grep .` fail and `!` inverted that into success, so the gate passed having disassembled nothing                                                                                                                                                                                                                                                                                                                                                                                       |
| No-SIMD guard, FP present                                        | build the same tree for `aarch64-unknown-none` (hard float)                                                                            | `error: FP/SIMD registers found`, on `v0`. The image carries 9 scalar `h` registers the earlier `[qv]` pattern ignored — on this tree they share lines with `v`, so the widened pattern adds coverage for a class (`fmov d0, x1` with no vector register) rather than a detection                                                                                                                                                                                                                                                                                                                                  |
| Board feature guard                                              | `cargo build --no-default-features`                                                                                                    | `no board selected — enable a board-* feature`; `make board-guard` asserts the refusal names the feature rather than cascading about a missing `bsp::board`                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `SCTLR_EL1` RES1 bits                                            | read the register back after boot and mask bits 11/20/22/23/28/29                                                                      | `SCTLR probe: 0x1005  RES1 set=0x0 of 0x30d00800` — six RES1 fields cleared by `msr sctlr_el1, xzr` and never restored. After the fix the same probe reads `0x30d01805`. QEMU only: the board has not been measured                                                                                                                                                                                                                                                                                                                                                                                                |
| `SCTLR_EL1` RES1, as a bring-up gate                             | restore `msr sctlr_el1, xzr` and boot `--features bringup`                                                                             | `SCTLR_EL1=0x1005 RES1=0x0/0x30d00800` then `selftest: FAIL SCTLR RES1`. The one-off probe that found this is now a gate: only _missing_ bits fail, because a part that forces its own RES1 bits is equally correct and worth knowing about                                                                                                                                                                                                                                                                                                                                                                        |
| No-SIMD guard, false positive                                    | add `ldr x0, =0x30d00800` to `boot.s` (a literal pool entry, no FP at all)                                                             | `error: FP/SIMD registers found`, pointing at `.word 0x30d00800` — `objdump` prints a literal pool's raw bytes even under `--no-show-raw-insn`, and the byte `d0` reads as the register. Widening the pattern to scalar FP is what made data sections start to matter; the earlier `[qv]` pattern could not hit it because `q` and `v` are not hex digits                                                                                                                                                                                                                                                          |
| GIC `enable` ordering                                            | reviewed against its own comment                                                                                                       | no red output — this one was found by reading. `GicV2::enable` masked the line _fourth_, after reprogramming group, priority, target and trigger, while the comment beside it said mask first and gave the right reason: with `enable_gic=1` the firmware has already programmed the distributor (ADR-0004), so a line can arrive live. No gate covers interrupt-controller programming order                                                                                                                                                                                                                      |
| IPC refusal counters, split                                      | the M4 gate asserted on a number covering three different things                                                                       | no red output. `ipc: refuse count=1 full=0 state=0` now separates an authority violation from a full mailbox and from a dead endpoint; the gate asserts the first is non-zero and the other two are zero. Before, filling a four-deep mailbox would have satisfied the forgery assertion                                                                                                                                                                                                                                                                                                                           |
| Pre-MMU path, direct branch                                      | `b switch_ttbr0` added to `_start`                                                                                                     | `_start calls 'switch_ttbr0': the pre-MMU window now includes code this check does not inspect`. The extractor harvested only `bl`, so a direct tail branch was neither audited nor refused and the gate printed clean having walked past it                                                                                                                                                                                                                                                                                                                                                                       |
| Restore-to-Pi-OS backup                                          | make the backup directory unwritable and run the copy                                                                                  | `could not back up … refusing to overwrite it`, exit 1. It was `cp … \|\| true` followed by the overwrite regardless — a failed backup destroyed the Harbor image with no copy anywhere, in the one script reached when something has already gone wrong                                                                                                                                                                                                                                                                                                                                                           |
| Cross-references                                                 | break a markdown link, cite an ADR number that does not exist, flip one status in the ADR index                                        | each named with its file: `links to 'verificaton.md', which does not exist`; `… is cited but no docs/adr/…-*.md exists`; `status is 'accepted', the index says 'proposed'`. All four classes were already correct — by attention, which does not survive a rename. The mutation's own number is left out of this row on purpose: writing it here makes this table a citation, and the gate is right to say so                                                                                                                                                                                                      |
| IPC waiter slot                                                  | let `park` overwrite the waiter instead of refusing                                                                                    | `a_second_waiter_is_refused_not_swapped_in` fails. Until this branch the only oracle for the whole IPC path was one `grep` over a boot log                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| IPC refusal counters, as tests                                   | count a full mailbox as an authority violation                                                                                         | two tests fail, including `a_full_mailbox_refuses_without_touching_the_authority_count` — the defect the M4 gate could not see                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Capability generation                                            | drop `ep.generation != cap.generation()` from the lookup                                                                               | `a_forged_capability_is_refused` and `a_stale_handle_from_a_recycled_slot_is_refused` both fail. Product path now also exercises stale handles after real `revoke_channel` (ADR-0032); host tests `revoke_*` and boot-check `ipc: release stale refused`                                                                                                                                                                                                                                                                                                                                                           |
| Parked stack, as tests                                           | overwrite the parked slot instead of handing it back; then stop parking on exit                                                        | `skipping_a_collection_point_is_counted_not_silent` fails on the first; four tests fail on the second, including `an_exit_into_a_task_that_has_never_run_still_parks` — the P0-2 ordering, which no boot performs and which nothing could drive before                                                                                                                                                                                                                                                                                                                                                             |
| Slot reuse before collection                                     | let `admit` hand out a slot whose stack is still parked                                                                                | `a_slot_whose_stack_is_still_parked_is_not_handed_out` fails. A case the old design avoided by accident — it detached the stack on exit — and that became reachable when the stack was left attached to its slot                                                                                                                                                                                                                                                                                                                                                                                                   |
| User-window text bound                                           | widen `bound_text_write` back to `pages * frame`                                                                                       | two tests fail, including `a_write_past_the_text_page_is_refused_even_though_the_window_is_bigger` — the P0-3 defect, where every offset in the window looked legal while the write went to page 0's physical address alone                                                                                                                                                                                                                                                                                                                                                                                        |
| User-window offset overflow                                      | `checked_add` back to a wrapping add                                                                                                   | `an_offset_that_would_overflow_is_refused_not_wrapped` fails: `usize::MAX + 1` wraps to zero and reads as a legal write at the start of the page                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |

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

**`make mutants`** runs it. The file list is the script's (`FILES` in
`scripts/host/run-mutants.sh`) and has grown past the ten modules this
paragraph first named: `taskcap`, `irqcap`, `reply`, `runqueue`, `irqwait` and
`capslots` joined it, because ADR-0058 §2 makes every module that decides
authority join the list in the commit it is born — `taskcap.rs` went unmutated
for a day (F-7) before that rule existed. Not wired into `make check`:
a full run is well over twenty minutes on a loaded machine,
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
_detected_ mutant — a hanging test is not a passing one — so the honest options
were to raise the timeout baseline from 1 to 2, or to remove the counter.

The counter went. `bind` now walks `entry.slots.iter().enumerate()`, which has
no `+=` to mutate, and a scoped re-run of `manifest.rs` alone reports **15
caught, 1 unviable, zero missed, zero timeouts**. So the baseline stays 10 and 1
rather than growing to accommodate a shape that did not need to exist.

That is the difference worth naming: raising a baseline records a weakness,
rewriting the loop removes one. The first is sometimes right — the six
`!mbox.live` arms are unreachable and stay — and this was not one of those
times.

### Fourth run, after ADR-0055/0057/0058: 361 caught, 17 missed, 1 timeout

The file list gained `taskcap.rs` and `irqcap.rs` (ADR-0058 §2: every module
that decides authority), and the script now refuses an artifact that did not
cover its own list — seen red against the manifest-only artifact the previous
scoped re-run had left behind. 420 mutants over 12 files.

The widened net earned its keep twice before the baseline settled:

- **Five encoders had no assembler-oracle row** — `encode_wait_irq_exit`,
  `encode_resolve_exit`, `encode_transfer_exit`, `encode_transfer_peer_exit`,
  `encode_recv_timeout_exit` all survived replacement by a constant array.
  Ten mutants, killed by adding the missing `llvm-mc` rows.
- **The band decode and cross-task revoke were untestable from an empty
  table** — taskcap/irqcap `lookup` band checks and `revoke_task`'s
  `task == id` conjunct survived because the older tests never held a live
  entry while probing a wrong-shaped id. Killed by
  `endpoint_shaped_id_never_hits_a_live_entry`,
  `low_shaped_id_never_hits_a_live_entry`,
  `revoke_task_leaves_other_tasks_caps_live`, and four new ipc tests
  (stale-generation revoke refusal with counter assertion, unknown-index
  counter, kill-only-the-named-channel, two auto-reap waiters in one
  release call).

The 17 that remain are argued in the script beside the baseline: the six
`!mbox.live` arms are now **twelve mutants on the same six-plus-two sites**
(this cargo-mutants emits `-=` and `*=` per `+=`), plus two release_holds
boundary guards, the three model-check-guarded `tasks::switch` mutants, three
equivalents (`1<<0`→`1>>0`; `|`→`^` on disjoint band bits, pinned by the
taskcap const assert), and irqcap's generation-0 skip, reachable only at a
u16 wrap that cannot occur while irqcap has no revoke. The one timeout is
`reset::partition`'s documented no-op-counter hang.

### Fifth run, after ADR-0059/0060/0061: 369 caught, 17 missed, 1 timeout

`reply.rs` joined the file list the commit it was born (ADR-0058 §2 — a new
module carrying the reply semantics is an authority module by definition). 387
mutants over 13 files. **Zero survivors in `reply.rs`** and zero in the new
`CapClass` decode; the 17 missed are the same justified set as the fourth run
(the baseline did not move), and the timeout is still `reset::partition`'s
documented hang.

### Sixth run, after ADR-0062: 428 caught, 19 missed, 1 timeout

`runqueue.rs` (now home of the task identity) and `irqwait.rs` (decides wake
delivery) joined the file list the commit the epoch landed — 505 mutants
over 15 files. The
widened scope surfaced ten new survivors; eight died to new tests (the
`to_raw`/`from_raw` transport layout, `capacity`, `is_pending` one past the
bound) and two are justified, both instances of already-argued classes:

- `epoch << 16 | slot` → `^` in `TaskId::to_raw` — equivalent: the two
  halves occupy disjoint bits, the same argument as the band mints.
- `task.slot() < MAX_TASK_IDS` → `<=` in `irqwait::signal` —
  defensive-unreachable: `arm` refuses those slots, so no armed entry can
  carry one.

The 17 from the fifth run are unchanged; baseline 17 → 19, both additions
named above. The timeout is still `reset::partition`'s documented hang.

### Seventh run, after ADR-0063: 446 caught, 19 missed, 1 timeout

`capslots.rs` joined the file list the commit it was born (ADR-0058 §2) —
525 mutants over 16 files. **Zero survivors in `capslots.rs`**: every slot
decision the extraction moved out of `sched` (resolve, install, the transfer
refusal order, the ADR-0055 band filter, drain) dies to a host test. The 19
missed are the sixth run's justified set unchanged, and the timeout is still
`reset::partition`'s documented hang.

### Eighth run, after ADR-0091/0092: 15 survivors that were nobody's new code

This one is the argument for running the gate on a cadence rather than when a
module is born.

`lifecycle.rs` joined the file list the commit it was born (ADR-0058 §2), which
meant running `make mutants` — and it came back **34 missed against a baseline
of 19**, with **zero** survivors in `lifecycle` and no other change in
`crates/`. The fifteen extra were not new. The baseline was last set by
`aca0d60` (ADR-0062); `cpu1_started` arrived with ADR-0076 and the steal
predicates with ADR-0083, so the whole of **K8 — queues, per-core timer,
EL0-on-CPU1, work stealing — landed `done (HW)` without the mutation gate ever
seeing it.** ADR-0058 §2 says a module joins the list the commit it is born; it
does not say the list must be _re-run_ when existing modules grow, and for K8
they grew a lot.

Fourteen died to tests written against them:

| Survivor                                                                        | Test that kills it                                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `start_cpu1`'s `idle[1] != IDLE` → `==`                                         | `cpu1_has_no_idle_identity_until_it_is_started` — a fresh table gets a real second identity, and the call is idempotent                                                                                                                                                                                                                                                                                                    |
| `start_cpu1`'s **other** `!=`, the `p.slot() != i` that skips the parked slot   | `cpu1_idle_does_not_take_a_slot_whose_stack_is_still_parked` — a slot is `Empty` the moment its task exits but its stack stays attached until collected, and handing that slot to CPU 1's idle would give idle a stack another task still holds. Every other test here runs with `parked == None`, which is why it survived twice: once the mutation gate never ran, and once I mistook it for the guard three lines above |
| `cpu1_started` → `true` / `false` / `!=` → `==` (3)                             | same test, asserting before and after                                                                                                                                                                                                                                                                                                                                                                                      |
| `set_stealeable` bounds and idle guards, `\|\|` → `&&` (2)                      | `set_stealeable_refuses_a_stale_id_an_out_of_range_one_and_idle` — each idle refused **on its own**, not only both at once                                                                                                                                                                                                                                                                                                 |
| `is_stealeable`'s `idx < N` → `<=`                                              | `is_stealeable_refuses_out_of_range_stale_and_idle` — the mutant indexes past the array, so the test fails by panic                                                                                                                                                                                                                                                                                                        |
| `can_steal_into` → `true`, its guard `\|\|` → `&&`, its probe `&&` → `\|\|` (4) | `can_steal_into_needs_an_empty_local_queue_and_a_ready_stealeable_peer` and `can_steal_into_ignores_a_stealeable_peer_that_is_not_ready` — Ready-but-unmarked and marked-but-Blocked are each refused                                                                                                                                                                                                                      |
| `for_each_ready`'s `(head + i) % CAP` → `/` and `+` → `*` (2)                   | `for_each_ready_walks_head_to_tail_including_across_the_wrap` — the steal probe only ever asked "is there one?", so the ring arithmetic was never observed in order                                                                                                                                                                                                                                                        |

Two are **equivalent**, and both for reasons already in this document:

- `Tasks::wake`, `id == idle[0] \|\| id == idle[1]` → `&&`. Idle is never
  `Blocked`, so the state check three lines down refuses the same ids the
  guard does. Same invariant as the three long-standing `switch` survivors.
- `Tasks::switch_on`, `current == idle && queues[cpu].is_empty()` → `\|\|`.
  `try_steal_into` opens with the _same_ emptiness check, so the extra call
  the mutant makes returns `false` on its own; and the other direction — a
  busy CPU with an empty queue — cannot happen, because idle is always
  exactly one of `current` or queued. The guard is a readable statement of
  intent rather than a load-bearing test, and the mutant proves it.

Baseline 19 → **21**, both additions named above.

### Ninth run, after ADR-0096/0097: 612 mutants, 21 survivors, the baseline holds

`loaderplan` joined the list the commit it was born (ADR-0058 §2), and this is
the first run recorded by `docs/mutation-stamp.toml` — the artefact
[ADR-0096](adr/0096-gates-that-do-not-depend-on-remembering.md)'s freshness
gate compares against.

It also found one real gap of its own: `loaderplan::plan` refused an output
buffer **exactly** as long as the table, and equal-length is the normal case —
`loader::load_all` sizes its buffer from `MAX_AGENTS` and a store cannot exceed
it. Every test written for that module passed a buffer with room to spare, so
the boundary was never touched. Killed by
`an_output_buffer_exactly_the_size_of_the_table_is_enough`.

**On running it in parallel.** The first attempt used `--jobs 6` and came back
with five timeouts against a baseline of one. Four were mutants the serial run
had caught — `ipc::release_holds`, `ipc::try_recv` twice, `tasks::switch_on`.
cargo-mutants derives its per-mutant timeout from an unmutated baseline it
measures while the machine is idle, and every parallel job then makes every
test slower than that measurement, so the number was a fact about a laptop
under load. `run-mutants.sh` now raises the floor to ten minutes per mutant
whenever `MUTANTS_JOBS` is set: the real hang (`reset::partition`) still hits
it, a merely slow test does not. Re-run at six jobs with the floor: 21
survivors, 1 timeout, 33 minutes instead of three hours.

This is the same correction [ADR-0087](adr/0087-oracle-waits-and-the-hosts-verdict.md)
made for the boot oracle, in a different gate: a verdict must not depend on how
busy the host was.

> The first version of this section said _thirteen_, and set the baseline from a
> count rather than from a run. The finished run returned 22, not 21: the
> fourteenth is the `p.slot() != i` above, which I had read as the same mutant as
> the `idle[1] != IDLE` guard three lines earlier. Corrected here rather than by
> raising the baseline to fit — the gate is what found it, which is the argument
> for running it.

**The lesson is the cadence, not the survivors.** Every one of these thirteen
was reachable by a test the day the code landed; what was missing was a run.
ADR-0058 §2's rule ("a module joins the list the commit it is born") is
necessary and was followed — `runqueue` and `irqwait` did join for ADR-0062 —
but the list is not the gate. Running it is.

### Tenth run, after ADR-0098: 621 mutants, and a timeout that was a laptop

The slot meter ([ADR-0098](adr/0098-slot-meter-measured.md)) moved
`tasks.rs`'s mutable surface, `make mutation-freshness` went red on it — 621
against a stamp of 612, which is the gate doing exactly its job on the commit
that armed it — and the run that followed found **no new survivor in the new
code**. Three of `note_occupancy`'s four mutants die: the body replaced by
`()`, and `>` replaced by `<` and by `==`. All three leave the watermark
below what the table actually held, and the tests assert the peak
**exactly** rather than as a lower bound, so all three fail.

The fourth is equivalent and joins the baseline (21 → 22):

- `note_occupancy`'s `live > self.peak` → `>=`. The mutant assigns the
  watermark the value it already holds. No reachable state distinguishes the
  two, and no test can, which is the definition being used here rather than an
  excuse for a missing one.

**The interesting part is how it was first reported.** The 68-minute serial
run filed that mutant as a **timeout**, not a survivor — two timeouts against
a baseline of one, which failed the target. Examined alone it re-ran in
**6 seconds**. Nothing about it is slow; the run had simply drifted past
cargo-mutants' auto-measured per-mutant timeout, which is taken once from an
unmutated baseline at the start. This laptop caps `make` at a single core, so
that measurement is a snapshot of one moment's scheduling and every later
mutant is racing a number taken under different conditions.

`run-mutants.sh` already had the cure and applied it too narrowly: the
ten-minute floor was raised **only when `MUTANTS_JOBS` was set**, on the
assumption that load comes from cargo-mutants' own parallelism. It comes from
the machine. The floor is now unconditional (300 s serial, 600 s parallel),
and the real hang it exists to catch — `reset::partition`'s no-op loop counter,
which never terminates — still hits it.

The re-run with the unconditional floor: **621 mutants, 22 survivors, one
timeout** — `reset::partition`, the loop counter that genuinely never
terminates. Its stamp reads `commit = 205f8ca`, three commits behind the code
it measured, because `run_commit` was captured before an 82-minute run instead
of when the stamp is written. Fixed in the same pass; the value is left as the
run produced it rather than edited by hand into looking right.

This is [ADR-0087](adr/0087-oracle-waits-and-the-hosts-verdict.md)'s rule for
the third time, in the third gate: **a verdict must not depend on how busy the
host was.** It was worth checking rather than believing — raising
`BASELINE_TIMEOUT` to 2 would have recorded a fact about a laptop as a fact
about the kernel, and hidden a genuine equivalent survivor behind it.

#### What mutation testing cannot reach here

`cargo-mutants` runs `-p kernel-core`. Everything in `src/` is outside it,
because it is not host-testable — which means the _kernel-side_ half of some
claims has no mutation coverage at all. The sharpest instance this section
used to name — `RecvError::Busy → Status::Busy`, two lines in `src/agent`
nothing mutated and nothing on the boot path reached — **no longer exists**:
ADR-0060 moved every reply mapping into `kernel_core::reply`, where it is
host-tested per outcome and mutated (zero survivors on the first run). What
remains outside the net is the marshalling residue in `src/agent` (register
reads, the one-arm-per-variant outcome conversions) and the rest of `src/` —
still named here rather than left to be inferred from a green run.

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

It runs on nightly, and that requirement is contained rather than avoided:
`make miri` is a `check` prerequisite, but nightly is needed **only** for that
target, so the kernel's own toolchain pin stays stable. A developer without it
opts out through `ALLOW_MIRI_SKIP=1`, loudly and on purpose; CI never does.
(This paragraph said the opposite — "a separate CI job and not part of
`make check`" — for as long as it took someone to read the Makefile beside it.
`doc-claims` compares the gate _list_, not prose about it, which is the blind
spot ADR-0058 §2 names.)

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

### Current Pi 4B product stamp (2026-08-14)

The product image (`--no-default-features --features board-rpi4`, store injected)
booted on a Raspberry Pi 4 Model B Rev 1.5 over a CP2104 UART at 115200 8N1.
The complete capture is `.serial-log/20260814-113438.log`; `make hw-check
TRANSCRIPT=.serial-log/20260814-113438.log` reports clean. It records
`src=dcc997cc`, `loader: store n=5 image`, `authority: bound console`,
`authority: 1 blob ok` / `2 blob-reply ok`, `blob: put ok`, `blob: got`,
wire bytes `N` and `S`, and `invariants: … slots=4/9`. Network stays
`authority: network vocabulary VACANT`. This pays ADR-0102 and ADR-0103 on
silicon and updates the ADR-0098 watermark for the five-agent composition.

This is product composition evidence, not a NIC claim. ADR-0105/0106 remain
proposed.

### Current Pi 4B oracle stamp (2026-08-14)

The freshly deployed oracle image booted on a Raspberry Pi 4 Model B Rev 1.5
over a CP2104 UART at 115200 8N1. The complete capture is
`.serial-log/20260814-020236.log`; `scripts/check/hw-transcript-check.sh`
reports clean. It records `smp: core1 alive`, `durable-media: boot=22
from=Previous part=0x7f slot=A seq=21`, the verified flush to `seq=22`, and
the full oracle sequence through stable tick reports (`overwrites=0`,
`abandoned=0`). The image reports `src=6b5f612f`; the subsequent HEAD
`791d39f` is a documentation-only merge, so the runtime source is explicitly
identified rather than implying an unbuilt HEAD image.

This is boot, SMP, EL0, IRQ, scheduler, IPC, and durable-media evidence only;
it does not claim Pi 4 GENET support. ADR-0105/0106 remain the boundary for
the still-unimplemented hardware NIC backend.

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

| Claim                       | Gate / evidence                                                          |
| --------------------------- | ------------------------------------------------------------------------ |
| Server up                   | `console-server: up` — QEMU + Pi 4B                                      |
| Product beacon              | `loader: beacon ran sends=2 refusals=0` + wire `H!` before the report    |
| Mute denial (oracle)        | `loader: mute ran sends=0 refusals=2`; refuse count=5                    |
| Console via SEND (not putc) | `el0-task: console sends=2`; `decode(2) == Unknown`                      |
| Product image               | `make product-builds` + `make product-boot-check` + `make oracle-census` |
| Payload still crosses EL0   | `*el0-ipc: got payload via EL0 recvs=1`                                  |

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

| Claim                       | Gate / evidence                                                                   |
| --------------------------- | --------------------------------------------------------------------------------- |
| Parks are counted           | Host tests; boot-check `sched: blocked=… block_events=…`                          |
| Orphan wait cancelled       | Boot-check + Pi 4B: `ipc: reaped cancelled` + `ipc: cancel issued cancel_events=` |
| Waiter cleared without send | Host test `clear_waiter_drops_the_parked_slot_without_a_send`                     |
| EL0 status                  | `Status::Cancelled = 5`; SECURITY authority table                                 |

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
Both were closed by the 2026-08-08 depth stamp (`ipc: auto-reaped
cancelled`, `ipc: release stale refused`, `el0-irq: woke` on silicon — see
the H1 sections above); what remains open lives in one place, the
[completeness roadmap](roadmap.md).

## Hardware evidence: K7 ASID slice + early-map retirement closed on silicon (2026-08-09)

| Claim                                           | Gate / evidence                                                                                                                                                                                                 |
| ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Early-map TLB residue retired at activate       | Red first: transcript `20260809-093312.log` — `el0: SVC`/`el0: FAULT`/`asid: dual` failing with instruction abort, permission fault L1 at the user window. Green after `retire_early_map`: transcript below     |
| ASID/nG regime works on silicon (ADR-0047/0050) | `asid: dual a=2 b=3 ok` — two ASes, distinct ASIDs, both enter EL0, no global TLBI between them                                                                                                                 |
| Whole oracle set holds on Pi 4B                 | `scripts/check/hw-transcript-check.sh` **clean** — every `boot-oracle.sh` assertion, including the ADR-0055/0057/0061/0062 refusal lines (`refused refusals=… detail=4`, stale task-cap refused, donor emptied) |

**Status: done (HW)** on Pi 4B, 2026-08-09 ~10:07 host time. Transcript:
`.serial-log/20260809-100645.log` (oracle `kernel8.img` @ `1843423`, which
carries ADR-0062 epoch identities and ADR-0063 capslots).

This stamp is **correctness**, not cost. Switch-cost lab (**K7-M**) and TTBR1
(**K7-T**) are governed by [ADR-0084](adr/0084-k7-residual-policy.md): measure
is optional lab evidence; TTBR1 stays deferred until a named trigger fires.
