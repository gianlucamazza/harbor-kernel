# Harbor

**A verified Rust kernel for Raspberry Pi 4** — package `harbor-kernel`.

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![CI](https://github.com/gianlucamazza/harbor-kernel/actions/workflows/ci.yml/badge.svg)](https://github.com/gianlucamazza/harbor-kernel/actions)

Repository: **https://github.com/gianlucamazza/harbor-kernel** · Tracking:
[issues](https://github.com/gianlucamazza/harbor-kernel/issues) ·
[project](https://github.com/users/gianlucamazza/projects/3)

## What Harbor is

Harbor aims to be an **agent-based microkernel** for the **Raspberry Pi 4
Model B**: isolated units that share no memory and interact only through
messages and capabilities. The name is the metaphor — a protected place where
bounded components operate and talk over controlled channels
([ADR-0007](docs/adr/0007-project-identity-harbor-kernel.md)).

**Today** it is a single-core bare-metal AArch64 kernel in Rust (`no_std`),
booting under the platform firmware. It runs cooperative EL1 tasks, IPC with
unforgeable caps, and EL0 agents in private address spaces whose authority is
named by capability slot. An agent can **wait** for a message, and a **loader**
creates agents from a manifest rather than from code compiled beside them. It is
**not** a finished agent OS: an agent cannot wait on an interrupt as a
first-class wake, it is not preempted, and the manifest a product image carries
is empty — the loader is real, and so far it is the boot oracle that gives it
something to load. Full product surface:
[`docs/architecture.md`](docs/architecture.md).

_Verified_ means claims in this README are backed by host tests, Miri on the
pure-logic `unsafe`, build-enforced invariants, a QEMU boot gate, and fault
probes on real silicon — with known blind spots in
[`docs/verification.md`](docs/verification.md) (notably: QEMU does not model
memory attributes the way Cortex-A72 does). Authority and isolation claims
(and their limits): [`SECURITY.md`](SECURITY.md).

## Status

**M0–M7 done (HW)** on Pi 4B, plus the loader and blocking receive. Bring-up
through EL0 authority-by-slot is stamped on silicon; the next work is product
surface on top of that boundary, not another foundation milestone.

| ID           | Deliverable                                                                                  | Status                   |
| ------------ | -------------------------------------------------------------------------------------------- | ------------------------ |
| M0–M2, P0–P4 | Boot, exceptions, MMU, heap, W^X, idle, layout gates                                         | **done (HW)**            |
| M3           | Cooperative EL1 tasks + guarded stacks                                                       | **done (HW)**            |
| M4           | IPC + capabilities (mailboxes, refuse, IRQ wake queue)                                       | **done (HW)**            |
| M5           | EL0 agents (private AS, `TTBR0`, SVC + fault probes)                                         | **done (HW)**            |
| M6           | Driver-as-agent (PL011 page map, FR, RX own, kill)                                           | **done (HW)**            |
| M7           | EL0 authority by capability slot; console cap denied by default; creator-handled agent fault | **done (HW)** 2026-08-07 |
| ADR-0022     | Blocking `SYS_RECV`: an agent waits; the masked region is one session step, not the session  | **done (HW)** 2026-08-07 |
| ADR-0021     | Agents as data: a manifest binds grants by index; the loader creates them                    | **done (HW)** 2026-08-07 |

Post-M6 product slices (EL0 IRQ resume, `SYS_PUTC`, PL011 RX-owned agent) closed
[issue #1](https://github.com/gianlucamazza/harbor-kernel/issues/1) on silicon
2026-08-06. M7 closed in one boot the next day, and the loader and the park in
one boot after that — evidence in
[`docs/verification.md`](docs/verification.md#hardware-evidence-the-loader-and-the-park-on-silicon-2026-08-07).

### Next

The next work is **M8: the console as an endpoint**, which retires the
transitional `SYS_PUTC` — and gives the product manifest its first inhabitant.

The ordered list, with the done-when criterion for each item, lives in
[`docs/architecture.md#roadmap`](docs/architecture.md#roadmap). It is not
duplicated here: this table used to be a second copy, and it is the copy that
went stale — it still listed blocking `SYS_RECV` as pending work on the day that
landed on silicon.

**Not goals** until their own ADR: preemption, SMP, high-half / `TTBR1`, ASID
production, USB host, full framebuffer, a long-running interactive echo agent
replacing the idle body. This is a **lab kernel**, not multi-tenant production
software ([`SECURITY.md`](SECURITY.md)).

## What exists

Boot to EL1, a mapped and protected address space, interrupts, a heap,
**cooperative tasks** (M3), IPC/caps (M4), **EL0 agents** (M5–M7), and an
interactive serial console.

| Area           | State                                                                                                                                                                                                     |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Boot           | EL2→EL1, softfloat, DTB pointer captured (mapped RO; board truth is BSP constants — ADR-0011)                                                                                                             |
| Memory         | Multi-level identity map, **W^X**, guarded kernel + exception stacks, runtime `map`/`unmap` (block split), TLB maintenance                                                                                |
| Allocation     | Free-list allocator behind `GlobalAlloc` — `Box`/`Vec` work                                                                                                                                               |
| Frames (M5)    | Named phys pool (ADR-0012); `AddressSpace` clone + user VA window sized **per agent** (`text_pages` executable, the rest stack); destroy returns frames                                                    |
| EL0 (M5)       | `enter`/`resume`/`end_session`, own `TTBR0`, SVC + fault probes (**done HW**) — ADR-0014                                                                                                                  |
| Agent shell    | `src/agent::Agent` owns AS; scheduled multi-SVC; concurrent dual-TCB (**done HW**); `SYS_PUTC` + IRQ resume (**done HW**); the EL1 mask is one enter/resume step, never the session (ADR-0022)            |
| Loader         | `kernel_core::manifest` + `src/bootstrap/loader.rs`: an agent is an image, a window geometry and a slot table; a slot names an **index** into the loader's caps, never a `CapId` (**done HW**) — ADR-0021 |
| Waiting        | `SYS_RECV` parks the agent and a peer's send wakes it; `SYS_TRY_RECV` is the non-blocking half and the only producer of `Status::Empty` (**done HW**) — ADR-0022                                          |
| Authority (M7) | Slot-indexed caps; console behind a cap denied by default; agent fault ends the session, creator decides the task (**done HW**)                                                                           |
| PL011 agent    | Page map (ADR-0013); FR; RX own (drain off, LBE inject, poll, kill restores) — M6 **done HW**                                                                                                             |
| Tasks (M3)     | Cooperative EL1 tasks, heap stacks with unmapped guards, voluntary yield, idle = console loop (ADR-0006)                                                                                                  |
| IPC (M4)       | Mailboxes + CapId send/recv; refuse counter; IRQ wake queue (ADR-0008); demo sender/receiver/forger                                                                                                       |
| Interrupts     | GICv2, arch timer PPI (absolute CVAL), PL011 RX via SPI, dispatch counters; lower-EL IRQ → agent when session unmasks                                                                                     |
| RNG            | Polled SoC RNG200 (raw FIFO words; no CSPRNG claim); soft bring-up line after MMU                                                                                                                         |
| Console        | Kernel TX shared; RX ring when kernel owns drain; agent may suspend drain + poll `DR`; idle `WFI`                                                                                                         |
| TFT (lab)      | Optional `--features debug-display`: SPI0 + ILI9486 status surface (regwidth-16 SKU; UART stays primary)                                                                                                  |
| Verification   | 284 host tests (unit, integration, doc), **bounded model checking** of the scheduler and authority core, Miri over the `unsafe`, layout validator, build gates, QEMU boot-check, fault-probed on hardware |

## What is not there yet

**Product path (next, above):** console-as-endpoint (M8), optional IRQ-wake RX.
An agent cannot yet treat an IRQ as a first-class wait, and UART RX ownership is
poll-based rather than the steady console path.

**The manifest a product image carries is empty.** Every entry is behind the
`oracle` feature, so the loader exists and loads nothing without it — `make
product-builds` prints the number rather than letting the loader's presence read
as a product that runs agents. M8's console endpoint is its first inhabitant.

**A parked agent is parked forever.** There is no timeout on `SYS_RECV` and
nothing reclaims a task waiting on an endpoint nobody holds the send end of;
see [`SECURITY.md`](SECURITY.md) for what that costs.

**Not started (and not claimed):** preemption, SMP, high-half/`TTBR1` kernel,
ASID production, Linux/POSIX compatibility, USB host, full framebuffer.

Longer surface and milestone criteria:
[`docs/architecture.md`](docs/architecture.md).

## Design in brief

- **Layers:** `arch` → `bsp` → `drivers` / `irq` → policy (`bootstrap`,
  `sched`, `ipc`, `agent`, `mm`, console). Import edges are gated by
  `make layering` — [`docs/architecture.md`](docs/architecture.md).
- **Memory:** early MMU before any Rust runs, then a fine **W^X** kernel map;
  user AS from a named frame pool — [`docs/mmu.md`](docs/mmu.md).
- **Execution:** cooperative only ([ADR-0006](docs/adr/0006-cooperative-execution-model.md));
  an agent is a task + private AS + EL0 session.
- **Authority:** capability slots (ADR-0017); console denied by default; on
  agent fault the kernel ends the session and the creator decides the task
  (ADR-0018) — [`SECURITY.md`](SECURITY.md).
- **Interrupts:** GICv2 + arch timer + UART RX — [`docs/interrupts.md`](docs/interrupts.md).
- **Blobs:** EEPROM + `start4.elf` only — [`docs/blobs.md`](docs/blobs.md).
- **Verification:** a claim is “done” only with a host/QEMU gate or a silicon
  stamp — [`docs/verification.md`](docs/verification.md).

## Layout

```
crates/kernel-core/  pure logic, host-tested — no MMIO, no assembly:
  authority     cap, ipc (mailboxes + endpoints), syscall, tasks (scheduler
                state machine), runqueue, wake, irqtable (dispatch + seal),
                rxline (who owns the UART, and in what order it changes hands)
  agent text    prog — the machine code EL0 agents run, checked against the
                assembly it documents by disassembling it (a64 builds the words)
  agent identity manifest — which agents exist and what each is granted; a slot
                carries an *index* into the loader's capabilities, never a CapId
  memory        paging, layout, frame, heap, bump
  hardware maths gic, uart, spi, rng, timer, reset (PM_RSTS decode), a64, poll,
                delay
  data          ring (SPSC), display, textgrid, font8x8
  tests/        public_api (the surface `src/` depends on) · model_sched and
                model_ipc (every operation sequence to a bound, against a
                reference implementation — see docs/verification.md)
src/
  arch/           facade (`cfg(target_arch)`); only path policy imports
  arch/aarch64/   MMIO, CPU/DAIF, cache, vectors, MMU, switch, CNTP, probe, bootinfo,
                  boot.s, link.ld (ISA-owned entry + memory map)
  irq/            IrqChip owner, mask, counters — the table itself is
                  `kernel_core::irqtable`
  drivers/        PL011, GICv2, RNG200, pm (reset cause); spi + ili9486
                  (+ delay/pin) behind debug-display
  bsp/            board feature select → `board` re-export
  bsp/rpi4/       memmap, GPIO, console, IRQ bind, RNG bind, pm bind; display bind (feature)
  bootstrap/      mod: boot sequence · loader: the manifest loop (product; the
                  table it reads is oracle-only) · console_loop: idle body ·
                  demos: smokes · selftest: gates
  sched/          TCBs, stacks, context switch, wake queue — the state machine is
                  `kernel_core::tasks` (ADR-0006/0008)
  ipc/            global, mask, hold check, wake — the authority surface is
                  `kernel_core::ipc` (M4)
  agent/          EL0 agent shell: AS + session, SVC dispatch, `SessionEnd`
  mm/             heap + GlobalAlloc, layout, address spaces, task stacks + guard unmap
  time.rs         tick counter
  status.rs       TFT status slots (debug-display only)
  console.rs      TX claim/install + RX ring + print / kprintln
  sync.rs         SyncCell for globals the IRQ path shares
  panic.rs        mask IRQ → steal console → halt (+ TFT banner if feature)
  main.rs         → bootstrap::run()
boot/config.txt   arm_64bit, enable_uart, enable_gic
docs/             architecture, arch-contract, porting, mmu, verification, adr, …
scripts/          fetch-blobs, deploy-sd, serial, serial-capture, restore-rpios-boot,
                  gate checks
```

Multi-arch **scaffold** (ADR-0015): AArch64 + Pi 4 only as product; see
[`docs/porting.md`](docs/porting.md) to add an ISA or board later.

## Requirements

| Need                                                             | Role                                                                         |
| ---------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Rust toolchain from [`rust-toolchain.toml`](rust-toolchain.toml) | Channel + `aarch64-unknown-none-softfloat`; `rustup` installs on first build |
| `llvm-objcopy`                                                   | ELF → `kernel8.img`                                                          |
| `qemu-system-aarch64`                                            | Emulated boot and `make boot-check` / `make check`                           |
| Nightly + Miri (optional locally)                                | `make miri`; skipped loudly if missing, required in CI                       |
| SD card + 3.3 V USB–serial                                       | Hardware run only                                                            |

Primary host for development is **Arch Linux**; any machine with the tools above
is fine.

## Build

```bash
make              # → target/aarch64-unknown-none-softfloat/release/kernel8.img
make check        # fmt-check test no-simd no-early-exclusives no-static-mut irq-scope boot-check bringup-builds debug-display-builds debug-builds board-guard product-builds miri doc-claims doc-symbols layering arch-board-free shellcheck xrefs, then clippy
make test         # host unit tests only
make fmt
```

The kernel is built **softfloat**: it contains no FP/SIMD, `CPACR_EL1.FPEN` is
left trapping, and `make no-simd` fails the build if a register ever appears in
the image. See `src/arch/aarch64/cpu.rs` for why.

## Flash and run

```bash
make blobs                              # verified against EXPECTED.sha256
make deploy SD_MOUNT=/run/media/$USER/boot   # default; must be a FAT boot partition
make serial SERIAL_DEV=/dev/ttyUSB0
```

Default images are **headless**: the SPI TFT status surface is opt-in. For the
Waveshare-class panel, pass the feature on **every** build and deploy (a plain
`make deploy` overwrites a glass image with one that never touches the HAT):

```bash
make FEATURES=debug-display deploy SD_MOUNT=/run/media/$USER/boot
# serial should show: display: ILI9486 up  cdiv=…  bit_clk=… Hz  status
```

To put Raspberry Pi OS boot files back on the card: `make restore-rpios`
(same mount-point checks as deploy).

**Serial (3.3 V only):** GND→GND, Pi TX (pin 8)→adapter RX, Pi RX (pin 10)→adapter TX.  
Details: [`docs/hardware.md`](docs/hardware.md).

### Expected console

**QEMU** (`make qemu`) — full boot, 2026-08-05. Emulation has no firmware DTB
(`x0` is not a device tree) and a different timer frequency than silicon.

```
Harbor: hello
EL1 · W^X map · heap · timer + UART RX IRQ · WFI idle
no DTB (x0 was 0x100); board constants are compiled in
MMU on  (W^X, guard page at 0xbb000, 102400 B of table arena left)
heap remaining = 67108864 bytes
rng200: unavailable (NotPresent)
CNTFRQ=62500000 Hz  timer=10 Hz  PPI=30
IRQs enabled (timer + UART RX)
idle: WFI when no RX/tick work
heap: Box at 0xb4010, Vec of 1024 sums to 523776
heap: 67100624 bytes free while held, 2 fragments
heap: 67108864 bytes free after drop (fully reclaimed), 1 fragments
unmap: page at 0xb5000 fault-ready
unmap: remapped and freed
split: page at 0x200000 split 1, remapped
sched: spawned task-a
sched: spawned task-b
arena: 1 splits, 24 tables free
task-a 0
task-b 0
task-a 1
task-b 1
task-a 2
task-b 2
task-a 3
task-b 3
ticks=10
ticks=20
...
```

Addresses in that transcript move whenever `.text` grows, so they are a sample
of one build rather than a promise. The lines are what matter: `fully
reclaimed`, `split 1` (a 2 MiB block really was rebuilt as a table), and the
two workers alternating. `rng200: unavailable (NotPresent)` is expected on
QEMU: the SoC block is not modelled; presence is soft-failed via `arch::probe`.

**Hardware** (Pi 4B) differs in the ways that matter for “is this silicon?”:

| Signal                               | Board                                                                                         |
| ------------------------------------ | --------------------------------------------------------------------------------------------- |
| DTB                                  | Present, then mapped RO (e.g. `DTB mapped: 61440 bytes at 0x2eff1000`)                        |
| `CNTFRQ`                             | `54000000` Hz (not TCG’s 62.5 MHz)                                                            |
| RNG200                               | `rng200: ok word=…` (raw sample; not a CSPRNG claim)                                          |
| TFT (`FEATURES=debug-display` + HAT) | `display: ILI9486 up  cdiv=…  bit_clk=… Hz  status` — navy fill + status banner (regwidth-16) |

Full HW evidence: stack split in
[`docs/verification.md`](docs/verification.md#hardware-evidence-stack-split-closed);
RNG + SPI0 + panel in
[`docs/verification.md`](docs/verification.md#rng200-and-spi0-hardware);
M7 authority boundary in
[`docs/verification.md`](docs/verification.md#hardware-evidence-m7-closed-on-silicon-2026-08-07).

Typed characters are echoed via the RX IRQ ring (main idles with `WFI` between
events). `fully reclaimed` is the line that distinguishes a real allocator from
the bump one it replaced. An `irq: unhandled=…` line only appears if the
dispatch counters move — on a healthy boot they stay at zero and stay quiet.

```bash
make qemu        # Ctrl-A x to quit
make qemu-gdb    # halted, waiting for gdb on :1234
```

### Bring-up self-test

The masked CNTP / HPPIR / IAR gates (M1) and a deliberate **task-stack guard
write** (M3 — panics with ESR/FAR on success) are behind a cargo feature, so
none of it is compiled into a production image:

```bash
cargo build --release --features bringup
```

Use a bringup image only to capture silicon evidence, then re-flash production.
See [`docs/verification.md`](docs/verification.md#m3-cooperative-tasks-hardware).

## Docs

| Doc                                            | Content                                          |
| ---------------------------------------------- | ------------------------------------------------ |
| [`docs/architecture.md`](docs/architecture.md) | Layers, agent model, milestones, **roadmap**     |
| [`SECURITY.md`](SECURITY.md)                   | Threat model, TCB, authority surface, residuals  |
| [`docs/verification.md`](docs/verification.md) | What is checked, and what each check is blind to |
| [`docs/mmu.md`](docs/mmu.md)                   | Two maps, regions, W^X, guard page               |
| [`docs/interrupts.md`](docs/interrupts.md)     | VBAR, GIC, timer, HW evidence                    |
| [`docs/boot-chain.md`](docs/boot-chain.md)     | ROM → EEPROM → start4 → kernel                   |
| [`docs/blobs.md`](docs/blobs.md)               | Closed firmware policy                           |
| [`docs/hardware.md`](docs/hardware.md)         | Pinout, GIC bases, optional SPI TFT HAT          |
| [`docs/porting.md`](docs/porting.md)           | Add an ISA or board (scaffold only today)        |
| [`docs/adr/`](docs/adr/README.md)              | Architecture decisions (ADRs)                    |
| [`docs/reviews/`](docs/reviews/)               | Multi-role analysis reports                      |

## Contributing

There is no external contributor process yet. Structural changes follow the
same discipline as the tree: multi-role review before milestones that move a
boundary ([ADR-0001](docs/adr/0001-multi-role-analysis.md)), and ADRs for
decisions that constrain the code. Local gate before a claim of “done”:

```bash
make check
```

## License

MIT ([`LICENSE-MIT`](LICENSE-MIT)) or Apache-2.0
([`LICENSE-APACHE`](LICENSE-APACHE)), at your option. Platform blobs stay under
upstream licenses — see [`docs/blobs.md`](docs/blobs.md).
