# Hardware — Raspberry Pi 4 Model B

## Target

| Item | Value |
|------|-------|
| Board | Raspberry Pi 4 Model B |
| SoC | BCM2711 |
| Cores | 4× Cortex-A72 (only core 0 active through M1) |
| Arch | AArch64, EL1 after bootstrap |
| Peripheral MMIO base | `0xFE00_0000` |

## Serial console

| Signal | Header pin | BCM GPIO | Function |
|--------|------------|----------|----------|
| TX (Pi → host) | 8 | GPIO 14 | PL011 UART0 TXD |
| RX (host → Pi) | 10 | GPIO 15 | PL011 UART0 RXD |
| GND | 6 (or any GND) | — | Common ground |

**Adapter must be 3.3 V logic.** 5 V UART adapters can damage the SoC.

| Parameter | Value |
|-----------|-------|
| Controller | ARM PL011 UART0 |
| MMIO base | `0xFE20_1000` |
| Clock (assumed) | 48 MHz with `enable_uart=1` |
| Baud | 115200 |
| Frame | 8N1 |
| Mode | TX polled; RX IRQ → kernel ring + WFI idle (P0) |

Host example:

```bash
make serial SERIAL_DEV=/dev/ttyUSB0
# or: picocom -b 115200 /dev/ttyUSB0
```

## Interrupt controller (M1)

| Block | Base |
|-------|------|
| GICD | `0xFF84_1000` |
| GICC | `0xFF84_2000` |

Requires `enable_gic=1` in `config.txt`. Timer IRQ: **PPI 30** (ARM physical
timer). Details: [`interrupts.md`](interrupts.md).

## Hardware RNG (RNG200)

On-die True Random Number Generator in the BCM2711 (iproc-rng200 family).
Harbor drives it with a **polled** driver (`drivers/rng200`); no IRQ path in v1.

| Item | Value |
|------|-------|
| Compatible | `brcm,bcm2711-rng200` |
| Register window | 0x28 bytes |
| ARM base (low peripheral) | `0xFE10_4000` (`memmap::RNG200_BASE`) |
| Legacy bus address | `0x7E10_4000` (not used; Harbor is low-peripheral) |
| GIC (unused in v1) | SPI 125 → absolute id **157** |

### Essential registers

| Offset | Name | Access | Role |
|--------|------|--------|------|
| 0x00 | RNG_CTRL | R/W | `RBGEN` (bit 0); sample divisor bits 13–20 |
| 0x04 | RNG_SOFT_RESET | W | Soft-reset RNG |
| 0x08 | RBG_SOFT_RESET | W | Soft-reset random bit generator |
| 0x0C | RNG_TOTAL_BIT_COUNT | R | Bits generated (warm-up) |
| 0x10 | RNG_TOTAL_BIT_COUNT_THRESHOLD | R/W | Threshold / IRQ |
| 0x18 | RNG_INT_STATUS | R/W1C | Health: NIST fail, master lockout, … |
| 0x1C | RNG_INT_ENABLE | R/W | IRQ mask (unused in v1) |
| 0x20 | RNG_FIFO_DATA | R | 32-bit random word |
| 0x24 | RNG_FIFO_COUNT | R/W | Low 8 bits = words available |

### Programming sequence (v1)

1. Soft-reset RBG then RNG; clear `RNG_INT_STATUS` (`0xFFFF_FFFF`).
2. Write `RNG_CTRL` with `RBGEN=1` and sample divisor `0x3` (common software
   convention, not a datasheet Fmax claim).
3. Wait until `RNG_TOTAL_BIT_COUNT` exceeds 16 (bounded spin).
4. Read: if `RNG_FIFO_COUNT & 0xFF` > 0, read `RNG_FIFO_DATA`.
5. On lockout / NIST fail in status → soft-reset + re-enable (limited budget).

Bootstrap logs one line (`rng200: ok word=…` or `rng200: unavailable (…)`)
after the MMU is on. Boot **never refuses** on logical failure (timeout /
health). Encodings are host-tested in `kernel-core::rng`.

**Interrupt:** GIC SPI 125 → absolute id **157** is reserved for a future
IRQ-driven path; v1 is polled and does not enable the line.

### QEMU

QEMU `raspi4b` (checked on 11.0.3 / upstream `bcm2838_peripherals`) does **not**
instantiate RNG200 at `0xFE10_4000`. Init probes the window with a recoverable
MMIO write (`arch::probe`); a missing backend becomes `RngError::NotPresent` and
a soft console line — not a panic. Silicon has the block and should log
`rng200: ok`. QEMU boot is not evidence of entropy quality.

### Limits and honesty

- Throughput is hundreds of Kibit/s class (divisor and load dependent).
- FIFO depth is small; empty FIFO is normal under burst reads.
- Internal health flags are not a substitute for entropy assessment.
- Public Broadcom docs do not describe the noise source or conditioning in detail.
- Harbor exposes **raw hardware words**, not a CSPRNG. Do not claim cryptographic
  quality or full min-entropy without offline evaluation.

The block needs no special clock/power setup beyond an already-running SoC. It
sits inside the existing 16 MiB peripheral Device map (`0xFE00_0000`).

## SD card

1. Partition 1: FAT32, bootable flag optional.
2. Files: see `docs/boot-chain.md`.
3. Deploy: `make blobs && make deploy SD_MOUNT=/path/to/boot`.

## Optional status display — Waveshare-class 3.5″ SPI TFT

**Status:** side-track in progress (not an M-milestone). Policy:
[ADR-0009](adr/0009-optional-spi-tft-debug-console.md) (**accepted**). Foundation in
tree behind `--features debug-display`: GPIO claim API, `SpiBus`/`SpiDevice`,
BCM2711 SPI0 polled master, BSP bind + empty-transfer smoke. ILI9486 / status
surface not yet.

This is **not** HDMI, DSI, or a VideoCore framebuffer. It is a GPIO HAT that
drives a TFT over **SPI0** with an **ILI9486** controller. Harbor will talk to
the panel from EL1 through a generic SPI + panel stack; the HAT is a **BSP
profile**, not the architecture name.

### Target SKU

| Item | Value |
|------|-------|
| Family | Waveshare 3.5inch RPi LCD (**A** / **B** / **C**) and pin-compatible clones (e.g. LCD wiki MPI3501) |
| Lab default profile | **Waveshare A** until a different PCB letter is confirmed in hand |
| Resolution | 480×320 |
| Colour | RGB565 (65 536 colours) |
| LCD controller | ILI9486 |
| Touch controller | XPT2046 (phase 2 only — second `SpiDevice` on the same bus) |
| Bus | SPI0, mode 0; start ≤16 MHz; raise only with silicon evidence (Fmax often quoted ~32 MHz) |
| v1 paint model | Cell/window updates — **not** a required full-frame 300 KiB buffer |

**A / B / C:** same pin class for documentation; **not** drop-in for init or
MADCTL. Each letter is a named BSP profile with its own declarative init table.
Model **C** high SPI rates are an optimisation after status text works.

### HAT pinout (physical 40-pin header numbers)

Source: Waveshare wiki for 3.5inch RPi LCD (A) and LCD wiki MPI3501 (same map).
Pins marked NC are unused by the HAT (UART on pins 8/10 remains free for the
serial console if you can reach the header edge or a pass-through).

| Phys | BCM GPIO | Symbol | Harbor use (v1) |
|------|----------|--------|-----------------|
| 1, 17 | 3V3 | 3.3 V | Power |
| 2, 4 | 5V | 5 V | Power |
| 6, 9, 14, 20, 25 | GND | Ground | Common ground |
| 11 | **17** | TP_IRQ | Phase 2 (touch) |
| 18 | **24** | LCD_RS | DC: 0 = command, 1 = data |
| 19 | **10** | LCD_SI / TP_SI | SPI0 MOSI (ALT0) |
| 21 | **9** | TP_SO | SPI0 MISO (touch; unused for LCD writes) |
| 22 | **25** | RST | LCD hardware reset |
| 23 | **11** | LCD_SCK / TP_SCK | SPI0 SCLK (ALT0) |
| 24 | **8** | LCD_CS | LCD chip select (active low; owned by `SpiDevice`, not the panel driver) |
| 26 | **7** | TP_CS | Phase 2 (touch CE1) |

No dedicated backlight GPIO on this pin map: the LED backlight is powered with
the board (≈150 mA class draw on 5 V per vendor FAQ).

**Conflict check with Harbor today:** console UART uses GPIO 14/15 (phys 8/10),
which the HAT leaves NC. GIC and PL011 bases are unchanged. SPI0 sits in the
low peripheral window already mapped as Device-nGnRnE (`0xFE00_0000`, 16 MiB) —
base **SPI0 = `0xFE20_4000`** (`bsp/rpi4/memmap::SPI0_BASE`). That 16 MiB
blanket is pre-existing (finding F26); new code must not widen it and should
use named bases rather than hard-coded offsets in panel code.

### What bare metal must do (v1) — correct path only

Policy detail: [ADR-0009](adr/0009-optional-spi-tft-debug-console.md).

1. General GPIO ownership + SPI0 pinmux (ALT0 on 9/10/11; outputs for CS/DC/RST).
2. Polled `SpiBus` + CS-scoped `SpiDevice` (shape aligned with embedded-hal 1.0;
   **local traits**, no e-hal crate dependency).
3. ILI9486: reset, **datasheet-first** declarative init (`Cmd` / `Data` /
   `DelayMs`), `PanelConfig` for MADCTL/BGR. Vendor trees (fbtft, Waveshare,
   TFT_eSPI) are cross-checks, not paste sources.
4. Timer-based delays (`CNTPCT`) for panel multi-ms waits — not CPU cycle spins.
5. Structured **status surface** (dirty cells); UART remains the full log stream.
6. Init **after** UART hello; `Result` on failure → one clear serial line, continue
   headless (never silent, never `unwrap` on the boot path).

**Rejected shortcuts:** bitbang SPI, monolithic vendor driver, full-frame blit
for status, CS toggled inside ILI9486, automatic serial mirror, paint from IRQ.

Touch (XPT2046) is phase 2 and requires the shared-bus `SpiDevice` shape above.

### References (external)

- ILI9486 datasheet (opcodes and timing floors — primary)
- Waveshare: [3.5inch RPi LCD (A)](https://www.waveshare.com/wiki/3.5inch_RPi_LCD_(A))
- LCD wiki MPI3501: [3.5inch RPi Display](https://www.lcdwiki.com/3.5inch_RPi_Display)
- Linux staging fbtft `fb_ili9486` — cross-check only

### Safety

- Power off before seating or removing the HAT.
- Do not hot-plug the 40-pin stack.
- Same 3.3 V GPIO rule as the serial adapter.

## Safety

- Power off before reseating the SD card or UART leads.
- Do not drive GPIO from 5 V.
- Secondary cores are parked in `wfe`; do not assume SMP readiness.
