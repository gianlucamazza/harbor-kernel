# rpi_minimal_agentic

Agent-based microkernel for the **Raspberry Pi 4 Model B**, written in Rust
(`no_std`), booting bare metal under the platform firmware.

**Milestone 2 (in tree):** M1 + identity **MMU** + **bump heap**.  
Validate on Pi: look for `MMU on` and `heap demo:` plus continuing `ticks=`.

## Design

- **Target:** agents, message passing, capabilities (see roadmap in
  [`docs/architecture.md`](docs/architecture.md)).
- **Layers:** `arch` → `bsp` → `drivers` / `irq` → `bootstrap` / `time` / console.
- **Interrupts:** [`docs/interrupts.md`](docs/interrupts.md).
- **Blobs:** EEPROM + `start4.elf` only — [`docs/blobs.md`](docs/blobs.md).

## Layout

```
src/
  arch/aarch64/   MMIO, CPU/DAIF, exception vectors, CNTP timer
  irq/            IrqChip trait, dispatch table
  drivers/        PL011, GICv2
  bsp/rpi4/       memmap, GPIO, console, IRQ bind (static GIC)
  bootstrap/      bring-up (production; optional selftest)
  time/           tick counter (plain u64 until MMU / M2)
  console.rs      acquire + print macros
  panic.rs        mask IRQ → UART → halt
  boot.s          EL2→EL1, CNTHCTL, BSS, stack
  main.rs         → bootstrap::run()
boot/config.txt   arm_64bit, enable_uart, enable_gic
docs/             architecture, interrupts, boot, blobs, hardware
scripts/          fetch-blobs, deploy-sd, serial, restore-rpios-boot
```

## Build (host: Arch Linux)

```bash
rustup target add aarch64-unknown-none
make              # → target/aarch64-unknown-none/release/kernel8.img
make check        # cargo check + clippy -D warnings
make fmt
```

## Flash and run

```bash
make blobs
make deploy SD_MOUNT=/run/media/$USER/bootfs
make serial SERIAL_DEV=/dev/ttyUSB0
```

**Serial (3.3 V only):** GND→GND, Pi TX (pin 8)→adapter RX, Pi RX (pin 10)→adapter TX.  
Details: [`docs/hardware.md`](docs/hardware.md).

### Expected console

```
rpi_minimal_agentic: hello
M2: MMU + heap + irq + CNTP
MMU on  (identity 2GiB RAM + device window)
heap remaining = …
CNTFRQ=54000000 Hz  timer=10 Hz  PPI=30
IRQs enabled
heap demo: alloc 64B at 0x…
ticks=10
ticks=20
...
```

Typed characters are echoed.

### Bring-up self-test

In `src/bootstrap/mod.rs`, set `BRINGUP_SELFTEST = true` to re-run soft/HPPIR/IAR
gates (used while debugging M1). Default is `false` for a short production boot.

## Docs

| Doc | Content |
|-----|---------|
| [`docs/architecture.md`](docs/architecture.md) | Layers, agent model, milestones |
| [`docs/interrupts.md`](docs/interrupts.md) | VBAR, GIC, timer, HW evidence |
| [`docs/boot-chain.md`](docs/boot-chain.md) | ROM → EEPROM → start4 → kernel |
| [`docs/blobs.md`](docs/blobs.md) | Closed firmware policy |
| [`docs/hardware.md`](docs/hardware.md) | Pinout, GIC bases |

## License

MIT OR Apache-2.0 (`Cargo.toml`). Platform blobs stay under upstream licenses.
