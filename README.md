# Harbor

**A verified Rust kernel for Raspberry Pi 4** — package `harbor-kernel`.

A bare-metal AArch64 kernel for the **Raspberry Pi 4 Model B**, written in Rust
(`no_std`), booting under the platform firmware. The long-term target is an
agent-based microkernel; see _What exists_ below for the difference between
that goal and the current tree.

*Verified* means the claims in this README are backed by host tests, Miri on
the pure-logic `unsafe`, build-enforced invariants, a QEMU boot gate, and
fault probes on real silicon — with known blind spots documented in
[`docs/verification.md`](docs/verification.md) (notably: QEMU does not model
memory attributes the way Cortex-A72 does).

**Status:** bring-up complete through M0–M2 and protection milestones P0–P4
(EL1, W^X map, heap, timer + UART RX IRQ, exception stack). **Next:** M3
cooperative tasks — roadmap in [`docs/architecture.md`](docs/architecture.md).

## What exists

Boot to EL1, a mapped and protected address space, interrupts, a heap, and an
interactive serial console.

| Area         | State                                                                                                                               |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| Boot         | EL2→EL1, softfloat, DTB pointer captured (not parsed)                                                                               |
| Memory       | Multi-level identity map, **W^X**, a guarded stack for the kernel and another for exceptions, runtime `map` with TLB maintenance                                    |
| Allocation   | Free-list allocator behind `GlobalAlloc` — `Box`/`Vec` work                                                                         |
| Interrupts   | GICv2, arch timer PPI, PL011 RX via SPI, dispatch counters                                                                          |
| Console      | Polled TX, interrupt-driven RX into a lock-free ring, `WFI` idle                                                                    |
| Verification | 102 host unit tests, Miri over the `unsafe`, a layout validator, build-enforced invariants, QEMU boot gate, fault-probed on hardware |

## What does not exist yet

No scheduler, no tasks, no address-space separation, no user mode, no IPC, no
capabilities — **none of the agent model is implemented.** Everything runs on
one core at EL1 in a single identity-mapped address space. The design those
words describe is the roadmap in
[`docs/architecture.md`](docs/architecture.md), not the code.

## Design

- **Layers:** `arch` → `bsp` → `drivers` / `irq` → `bootstrap` / `time` / console.
- **Memory:** early MMU before Rust, then a fine W^X kernel map — [`docs/mmu.md`](docs/mmu.md).
- **Interrupts:** [`docs/interrupts.md`](docs/interrupts.md).
- **Blobs:** EEPROM + `start4.elf` only — [`docs/blobs.md`](docs/blobs.md).

## Layout

```
crates/kernel-core/  pure logic, unit-tested on the host:
                     paging, allocators, GIC maths, SPSC ring, runqueue (M3)
src/
  arch/aarch64/   MMIO, CPU/DAIF, cache maintenance, vectors, MMU, CNTP, bootinfo
  irq/            IrqChip trait, dispatch table, counters
  drivers/        PL011, GICv2
  bsp/rpi4/       memmap, GPIO, console, IRQ bind (static GIC)
  bootstrap/      mod: boot sequence · console_loop: what the machine does · selftest: gates
  mm/             heap + GlobalAlloc, layout: regions and permissions
  time/           tick counter
  console.rs      TX claim + RX ring + print macros
  sync.rs         SyncCell for globals the IRQ path shares
  panic.rs        mask IRQ → steal console → halt
  boot.s          DTB pointer, EL2→EL1, early MMU, BSS, stack
  main.rs         → bootstrap::run()
boot/config.txt   arm_64bit, enable_uart, enable_gic
link.ld           load address, stack/guard, table arena
docs/             architecture, mmu, verification, interrupts, boot, blobs, hardware, adr
scripts/          fetch-blobs, deploy-sd, serial, restore-rpios-boot, gate checks
```

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
make check        # fmt-check test no-simd no-early-exclusives boot-check bringup-builds miri doc-claims layering, then clippy
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

**QEMU** (`make qemu`) — full current boot, 2026-08-04. Emulation has no firmware
DTB (`x0` is not a device tree) and a different timer frequency than silicon.

```
Harbor: hello
EL1 · W^X map · heap · timer + UART RX IRQ · WFI idle
no DTB (x0 was 0x100); board constants are compiled in
MMU on  (W^X, guard page at 0xa1000, 40960 B of table arena left)
heap remaining = 67108864 bytes
CNTFRQ=62500000 Hz  timer=10 Hz  PPI=30
IRQs enabled (timer + UART RX)
idle: WFI when no RX/tick work
heap: Box at 0xb2010, Vec of 1024 sums to 523776
heap: 67100624 bytes free while held, 2 fragments
heap: 67108864 bytes free after drop (fully reclaimed), 1 fragments
ticks=10
ticks=20
...
```

**Hardware** (Pi 4B, same tree, 2026-08-04) differs in the ways that matter for
“is this silicon?”:

| Signal | Board |
| ------ | ----- |
| DTB | Present, then mapped RO (e.g. `DTB mapped: 61440 bytes at 0x2eff1000`) |
| `CNTFRQ` | `54000000` Hz (not TCG’s 62.5 MHz) |
| Guard page | `0xa1000` (post exception-stack split) |

Full HW evidence (boot, overflow probe, W^X) lives in
[`docs/verification.md`](docs/verification.md#hardware-evidence-stack-split-closed).

Typed characters are echoed via the RX IRQ ring (main idles with `WFI` between
events). `fully reclaimed` is the line that distinguishes a real allocator from
the bump one it replaced. An `irq: unhandled=…` line only appears if the
dispatch counters move — on a healthy boot they stay at zero and stay quiet.

```bash
make qemu        # Ctrl-A x to quit
make qemu-gdb    # halted, waiting for gdb on :1234
```

### Bring-up self-test

The masked CNTP / HPPIR / IAR gates used to debug the M1 interrupt path are
behind a cargo feature, so none of it — including the raw GIC accessors it
needs — is compiled into a production image:

```bash
cargo build --release --features bringup
```

## Docs

| Doc                                            | Content                                          |
| ---------------------------------------------- | ------------------------------------------------ |
| [`docs/architecture.md`](docs/architecture.md) | Layers, agent model, milestones                  |
| [`docs/mmu.md`](docs/mmu.md)                   | Two maps, regions, W^X, guard page               |
| [`docs/verification.md`](docs/verification.md) | What is checked, and what each check is blind to |
| [`docs/interrupts.md`](docs/interrupts.md)     | VBAR, GIC, timer, HW evidence                    |
| [`docs/boot-chain.md`](docs/boot-chain.md)     | ROM → EEPROM → start4 → kernel                   |
| [`docs/blobs.md`](docs/blobs.md)               | Closed firmware policy                           |
| [`docs/hardware.md`](docs/hardware.md)         | Pinout, GIC bases                                |
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
