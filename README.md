# Harbor

**A verified Rust kernel for Raspberry Pi 4** — package `harbor-kernel`.

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![CI](https://github.com/gianlucamazza/harbor-kernel/actions/workflows/ci.yml/badge.svg)](https://github.com/gianlucamazza/harbor-kernel/actions)

Repository: **https://github.com/gianlucamazza/harbor-kernel** · Tracking:
[issues](https://github.com/gianlucamazza/harbor-kernel/issues) ·
[project](https://github.com/users/gianlucamazza/projects/3)

A bare-metal AArch64 kernel for the **Raspberry Pi 4 Model B**, written in Rust
(`no_std`), booting under the platform firmware. The long-term target is an
agent-based microkernel; see _What exists_ below for the difference between
that goal and the current tree.

*Verified* means the claims in this README are backed by host tests, Miri on
the pure-logic `unsafe`, build-enforced invariants, a QEMU boot gate, and
fault probes on real silicon — with known blind spots documented in
[`docs/verification.md`](docs/verification.md) (notably: QEMU does not model
memory attributes the way Cortex-A72 does).

**Status:** M0–M6 core **done (HW)** on Pi 4B through multi-agent shell and
multi-SVC resume
([`docs/verification.md`](docs/verification.md)). Post-M6 product slices —
EL0 IRQ resume, `SYS_PUTC`, PL011 **RX-owned agent** (poll + LBE) — **done
(QEMU)**; **HW stamp open**
([issue #1](https://github.com/gianlucamazza/harbor-kernel/issues/1)).
Roadmap: [`docs/architecture.md`](docs/architecture.md#roadmap).

## What exists

Boot to EL1, a mapped and protected address space, interrupts, a heap,
**cooperative tasks** (M3), and an interactive serial console.

| Area         | State                                                                                                                               |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| Boot         | EL2→EL1, softfloat, DTB pointer captured (mapped RO; board truth is BSP constants — ADR-0011)                                       |
| Memory       | Multi-level identity map, **W^X**, guarded kernel + exception stacks, runtime `map`/`unmap` (block split), TLB maintenance                                    |
| Allocation   | Free-list allocator behind `GlobalAlloc` — `Box`/`Vec` work                                                                         |
| Frames (M5)  | Named phys pool (ADR-0012); `AddressSpace` clone + user VA window; destroy returns frames                                           |
| EL0 (M5)     | `enter`/`resume`/`end_session`, own `TTBR0`, SVC + fault probes (**done HW**) — ADR-0014                                           |
| EL0 shell    | Scheduled agent: ping/refuse/exit, multi-SVC resume (**done HW**); `SYS_PUTC`, IRQ resume (**done QEMU**)                           |
| Agent shell  | `src/agent::Agent` owns AS; concurrent dual-TCB (**done HW**)                                                                      |
| PL011 agent  | Page map (ADR-0013); FR; RX own (drain off, LBE inject, poll, kill restores) — M6 v1 **HW**, own **QEMU**                           |
| Tasks (M3)   | Cooperative EL1 tasks, heap stacks with unmapped guards, voluntary yield, idle = console loop (ADR-0006)                            |
| IPC (M4)     | Mailboxes + CapId send/recv; refuse counter; IRQ wake queue (ADR-0008); demo sender/receiver/forger                                 |
| Interrupts   | GICv2, arch timer PPI (absolute CVAL), PL011 RX via SPI, dispatch counters; lower-EL IRQ → agent when session unmasks              |
| RNG          | Polled SoC RNG200 (raw FIFO words; no CSPRNG claim); soft bring-up line after MMU                                                   |
| Console      | Kernel TX shared; RX ring when kernel owns drain; agent may suspend drain + poll `DR`; idle `WFI`                                   |
| TFT (lab)    | Optional `--features debug-display`: SPI0 + ILI9486 status surface (regwidth-16 SKU; UART stays primary)                            |
| Verification | 265 host tests (unit, integration, doc), Miri over the `unsafe`, layout validator, build gates, QEMU boot-check, fault-probed on hardware                 |

## What does not exist yet

No preemption, no SMP, no high-half/`TTBR1` kernel, no ASID production, no
UART-IRQ-to-EL0 as the steady console path (ownership is poll-based). Longer
product surface: [`docs/architecture.md`](docs/architecture.md).

## Design

- **Layers:** `arch` → `bsp` → `drivers` / `irq` → `bootstrap` / `time` / console.
- **Memory:** early MMU before Rust, then a fine W^X kernel map — [`docs/mmu.md`](docs/mmu.md).
- **Interrupts:** [`docs/interrupts.md`](docs/interrupts.md).
- **Blobs:** EEPROM + `start4.elf` only — [`docs/blobs.md`](docs/blobs.md).

## Layout

```
crates/kernel-core/  pure logic, host-tested — no MMIO, no assembly:
  authority     cap, ipc (mailboxes + endpoints), syscall, tasks (scheduler
                state machine), runqueue, wake, irqtable (dispatch + seal),
                rxline (who owns the UART, and in what order it changes hands)
  memory        paging, layout, frame, heap, bump
  hardware maths gic, uart, spi, rng, timer, reset (PM_RSTS decode), a64, poll,
                delay
  data          ring (SPSC), display, textgrid, font8x8
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
  bootstrap/      mod: boot sequence · console_loop: idle body · demos: smokes ·
                  selftest: gates
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

| Need | Role |
| ---- | ---- |
| Rust toolchain from [`rust-toolchain.toml`](rust-toolchain.toml) | Channel + `aarch64-unknown-none-softfloat`; `rustup` installs on first build |
| `llvm-objcopy` | ELF → `kernel8.img` |
| `qemu-system-aarch64` | Emulated boot and `make boot-check` / `make check` |
| Nightly + Miri (optional locally) | `make miri`; skipped loudly if missing, required in CI |
| SD card + 3.3 V USB–serial | Hardware run only |

Primary host for development is **Arch Linux**; any machine with the tools above
is fine.

## Build

```bash
make              # → target/aarch64-unknown-none-softfloat/release/kernel8.img
make check        # fmt-check test no-simd no-early-exclusives boot-check bringup-builds debug-display-builds debug-builds board-guard miri doc-claims layering arch-board-free shellcheck xrefs, then clippy
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

| Signal | Board |
| ------ | ----- |
| DTB | Present, then mapped RO (e.g. `DTB mapped: 61440 bytes at 0x2eff1000`) |
| `CNTFRQ` | `54000000` Hz (not TCG’s 62.5 MHz) |
| RNG200 | `rng200: ok word=…` (raw sample; not a CSPRNG claim) |
| TFT (`FEATURES=debug-display` + HAT) | `display: ILI9486 up  cdiv=…  bit_clk=… Hz  status` — navy fill + status banner (regwidth-16) |

Full HW evidence: stack split in
[`docs/verification.md`](docs/verification.md#hardware-evidence-stack-split-closed);
RNG + SPI0 + panel in
[`docs/verification.md`](docs/verification.md#rng200-and-spi0-hardware).

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
| [`docs/architecture.md`](docs/architecture.md) | Layers, agent model, milestones                  |
| [`docs/mmu.md`](docs/mmu.md)                   | Two maps, regions, W^X, guard page               |
| [`docs/verification.md`](docs/verification.md) | What is checked, and what each check is blind to |
| [`docs/interrupts.md`](docs/interrupts.md)     | VBAR, GIC, timer, HW evidence                    |
| [`docs/boot-chain.md`](docs/boot-chain.md)     | ROM → EEPROM → start4 → kernel                   |
| [`docs/blobs.md`](docs/blobs.md)               | Closed firmware policy                           |
| [`docs/hardware.md`](docs/hardware.md)         | Pinout, GIC bases, optional SPI TFT HAT          |
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
