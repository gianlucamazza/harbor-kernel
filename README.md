# rpi_minimal_agentic

A bare-metal AArch64 kernel for the **Raspberry Pi 4 Model B**, written in Rust
(`no_std`), booting under the platform firmware. The long-term target is an
agent-based microkernel; see _What exists_ below for the difference between
that goal and the current tree.

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
| Verification | 84 host unit tests, Miri over the `unsafe`, a layout validator, build-enforced invariants, QEMU boot gate, fault-probed on hardware |

## What does not exist yet

No scheduler, no tasks, no address-space separation, no user mode, no IPC, no
capabilities — **none of the agent model is implemented.** Everything runs on
one core at EL1 in a single identity-mapped address space. The design those
words describe is the roadmap in
[`docs/architecture.md`](docs/architecture.md), not the code.

## Design

- **Layers:** `arch` → `bsp` → `drivers` / `irq` → `bootstrap` / `time` / console.
- **Memory:** [`docs/mmu.md`](docs/mmu.md).
- **Interrupts:** [`docs/interrupts.md`](docs/interrupts.md).
- **Blobs:** EEPROM + `start4.elf` only — [`docs/blobs.md`](docs/blobs.md).

## Layout

```
crates/kernel-core/  pure logic, unit-tested on the host:
                     paging encodings, allocators, GIC register maths, SPSC ring
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
  boot.s          DTB pointer, EL2→EL1, CNTHCTL, BSS, stack
  main.rs         → bootstrap::run()
boot/config.txt   arm_64bit, enable_uart, enable_gic
docs/             architecture, mmu, interrupts, boot, blobs, hardware
scripts/          fetch-blobs, deploy-sd, serial, restore-rpios-boot
```

## Build (host: Arch Linux)

The toolchain file pins the channel and the `aarch64-unknown-none-softfloat`
target, so `rustup` installs what is needed on first build.

```bash
make              # → target/aarch64-unknown-none-softfloat/release/kernel8.img
make check        # fmt-check test no-simd no-early-exclusives boot-check bringup-builds miri doc-claims, then clippy
make test         # host unit tests only
make fmt
```

The kernel is built **softfloat**: it contains no FP/SIMD, `CPACR_EL1.FPEN` is
left trapping, and `make no-simd` fails the build if a register ever appears in
the image. See `src/arch/aarch64/cpu.rs` for why.

## Flash and run

```bash
make blobs                                    # verified against EXPECTED.sha256
make deploy SD_MOUNT=/run/media/$USER/bootfs  # refuses anything not a FAT boot partition
make serial SERIAL_DEV=/dev/ttyUSB0
```

**Serial (3.3 V only):** GND→GND, Pi TX (pin 8)→adapter RX, Pi RX (pin 10)→adapter TX.  
Details: [`docs/hardware.md`](docs/hardware.md).

### Expected console

```
rpi_minimal_agentic: hello
EL1 · W^X map · heap · timer + UART RX IRQ · WFI idle
DTB at 0x2eff1f00
MMU on  (W^X, guard page at 0x9a000, 40960 B of table arena left)
heap remaining = 67108864 bytes
CNTFRQ=54000000 Hz  timer=10 Hz  PPI=30
IRQs enabled (timer + UART RX)
idle: WFI when no RX/tick work
heap: Box at 0xab010, Vec of 1024 sums to 523776
heap: 67100624 bytes free while held, 2 fragments
heap: 67108864 bytes free after drop (fully reclaimed), 1 fragments
ticks=10
ticks=20
...
```

That transcript is from a Raspberry Pi 4B, not an emulator: `CNTFRQ` is the
board's real 54 MHz, and the DTB address is the one this firmware passes.

It is also the last hardware boot **before** the exception-stack split, so the
addresses below the table arena have since moved — the guard page is now at
`0xa1000`, and there are two. Rather than paste emulator output and call it a
board, the transcript stays as recorded until the next hardware run; see
[the open item in `verification.md`](docs/verification.md#open-what-has-not-been-run-on-hardware).

Typed characters are echoed via the RX IRQ ring (main idles with `WFI` between
events). `fully reclaimed` is the line that distinguishes a real allocator from
the bump one it replaced. An `irq: unhandled=…` line only appears if the
dispatch counters move — on a healthy boot they stay at zero and stay quiet.

The same boot runs under emulation, which is the fast way to check a change:

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

## License

MIT ([`LICENSE-MIT`](LICENSE-MIT)) or Apache-2.0
([`LICENSE-APACHE`](LICENSE-APACHE)), at your option. Platform blobs stay under
upstream licenses — see [`docs/blobs.md`](docs/blobs.md).
