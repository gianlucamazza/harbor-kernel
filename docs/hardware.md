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

## SD card

1. Partition 1: FAT32, bootable flag optional.
2. Files: see `docs/boot-chain.md`.
3. Deploy: `make blobs && make deploy SD_MOUNT=/path/to/boot`.

## Optional debug display — Waveshare 3.5″ RPi LCD (SPI)

**Status:** planned side-track (not an M-milestone). Policy:
[ADR-0009](adr/0009-optional-spi-tft-debug-console.md). No driver in the tree yet.

This is **not** HDMI, DSI, or a VideoCore framebuffer. It is a GPIO HAT that
drives a TFT over **SPI0** with an **ILI9486** controller. Linux users often
install `fbtft` / Waveshare overlays; Harbor will talk to the panel directly
from EL1 when the optional cargo feature is enabled.

### Target SKU

| Item | Value |
|------|-------|
| Family | Waveshare 3.5inch RPi LCD (**A** / **B** / **C**) and pin-compatible clones (e.g. LCD wiki MPI3501) |
| Lab default | **A** until a different PCB letter is confirmed in hand |
| Resolution | 480×320 |
| Colour | RGB565 (65 536 colours) |
| LCD controller | ILI9486 |
| Touch controller | XPT2046 (out of scope for v1 — pixels only) |
| Bus | SPI0, mode 0; start at 8–16 MHz (panel Fmax often quoted ~32 MHz) |
| Full frame | 480×320×2 = **300 KiB** |

**A / B / C:** treat as the same pin class for documentation, but **do not**
assume identical init sequences or MADCTL rotation. Confirm the letter on the
silkscreen before claiming HW evidence. Model **C** advertises higher SPI
rates; that is an optimisation after a working text console, not a bring-up
requirement.

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
| 24 | **8** | LCD_CS | LCD chip select (active low; SPI0 CE0) |
| 26 | **7** | TP_CS | Phase 2 (touch CE1) |

No dedicated backlight GPIO on this pin map: the LED backlight is powered with
the board (≈150 mA class draw on 5 V per vendor FAQ).

**Conflict check with Harbor today:** console UART uses GPIO 14/15 (phys 8/10),
which the HAT leaves NC. GIC and PL011 bases are unchanged. SPI0 sits in the
low peripheral window already mapped as Device-nGnRnE (`0xFE00_0000`, 16 MiB) —
base **SPI0 = `0xFE20_4000`** (to be named in `bsp/rpi4/memmap` when the driver
lands).

### What bare metal must do (v1)

1. Pinmux SPI0 ALT0 on GPIO 9/10/11; GPIO output on 8 (CS), 24 (DC), 25 (RST).
2. Polled SPI0 transfers (no DMA, no SPI IRQ in v1).
3. ILI9486 reset + init sequence (port from open sources: Linux `fb_ili9486`,
   Waveshare / goodtft LCD-show, TFT_eSPI — not a closed blob RE exercise).
4. Partial window pixel writes for a **text cell console** (not full-frame
   refresh on every log line).
5. Init **after** UART hello; on failure, log once on serial and continue
   headless.

Touch (XPT2046 on the same SPI bus, separate CS) is explicitly **phase 2**.

### References (external)

- Waveshare: [3.5inch RPi LCD (A)](https://www.waveshare.com/wiki/3.5inch_RPi_LCD_(A))
- LCD wiki MPI3501: [3.5inch RPi Display](https://www.lcdwiki.com/3.5inch_RPi_Display)
- Linux staging fbtft: `fb_ili9486` init sequences

### Safety

- Power off before seating or removing the HAT.
- Do not hot-plug the 40-pin stack.
- Same 3.3 V GPIO rule as the serial adapter.

## Safety

- Power off before reseating the SD card or UART leads.
- Do not drive GPIO from 5 V.
- Secondary cores are parked in `wfe`; do not assume SMP readiness.
