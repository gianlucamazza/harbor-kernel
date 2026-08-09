# Hardware — Raspberry Pi 4 Model B

## Target

| Item                 | Value                                         |
| -------------------- | --------------------------------------------- |
| Board                | Raspberry Pi 4 Model B                        |
| SoC                  | BCM2711                                       |
| Cores                | 4× Cortex-A72 (only core 0 active through M1) |
| Arch                 | AArch64, EL1 after bootstrap                  |
| Peripheral MMIO base | `0xFE00_0000`                                 |

## Serial console

Harbor’s console is **only** the on-chip **PL011 UART0** on the 40-pin header.
There is no USB host / CDC stack in the kernel today, so a USB–serial dongle
plugged into a **Pi USB port is not a console** for Harbor (it is invisible to
the bare-metal image). Lab path:

```text
[PC] USB ──► 3.3 V adapter ── TX ──► Pi header 10 (GPIO 15 RX)
                           ── RX ◄── Pi header 8  (GPIO 14 TX)
                           ── GND ── Pi GND
                           (do not wire adapter VCC → back-feed)
```

| Signal         | Header pin     | BCM GPIO | Function        |
| -------------- | -------------- | -------- | --------------- |
| TX (Pi → host) | 8              | GPIO 14  | PL011 UART0 TXD |
| RX (host → Pi) | 10             | GPIO 15  | PL011 UART0 RXD |
| GND            | 6 (or any GND) | —        | Common ground   |

**Adapter must be 3.3 V logic.** 5 V UART adapters can damage the SoC.

| Parameter       | Value                                                                                                                                                            |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Controller      | ARM PL011 UART0                                                                                                                                                  |
| MMIO base       | `0xFE20_1000`                                                                                                                                                    |
| Clock (assumed) | 48 MHz with `enable_uart=1`                                                                                                                                      |
| Baud            | 115200                                                                                                                                                           |
| Frame           | 8N1                                                                                                                                                              |
| Mode            | TX polled (kernel-owned). Default: RX IRQ → kernel ring + WFI idle. Agent RX own: drain suspended, PL011 RX IRQs masked, EL0 polls `DR` (LBE self-test on QEMU). |

Host example:

```bash
make serial SERIAL_DEV=/dev/ttyUSB0
# or: picocom -b 115200 /dev/ttyUSB0
```

A second, identical dongle on the Pi’s USB port (or two dongles null-modemed
to each other) does **not** replace the GPIO path above. That second device is
only useful under an OS with USB host drivers (e.g. Raspberry Pi OS), not under
Harbor. Capture gotchas (one reader, no VCC, unplug on power cycle):
[`verification.md`](verification.md#serial-capture).

## Interrupt controller (M1)

| Block | Base          |
| ----- | ------------- |
| GICD  | `0xFF84_1000` |
| GICC  | `0xFF84_2000` |

Requires `enable_gic=1` in `config.txt`. Timer IRQ: **PPI 30** (ARM physical
timer). Details: [`interrupts.md`](interrupts.md).

## Hardware RNG (RNG200)

On-die True Random Number Generator in the BCM2711 (iproc-rng200 family).
Harbor drives it with a **polled** driver (`drivers/rng200`); no IRQ path in v1.

| Item                      | Value                                              |
| ------------------------- | -------------------------------------------------- |
| Compatible                | `brcm,bcm2711-rng200`                              |
| Register window           | 0x28 bytes                                         |
| ARM base (low peripheral) | `0xFE10_4000` (`memmap::RNG200_BASE`)              |
| Legacy bus address        | `0x7E10_4000` (not used; Harbor is low-peripheral) |
| GIC (unused in v1)        | SPI 125 → absolute id **157**                      |

### Essential registers

| Offset | Name                          | Access | Role                                       |
| ------ | ----------------------------- | ------ | ------------------------------------------ |
| 0x00   | RNG_CTRL                      | R/W    | `RBGEN` (bit 0); sample divisor bits 13–20 |
| 0x04   | RNG_SOFT_RESET                | W      | Soft-reset RNG                             |
| 0x08   | RBG_SOFT_RESET                | W      | Soft-reset random bit generator            |
| 0x0C   | RNG_TOTAL_BIT_COUNT           | R      | Bits generated (warm-up)                   |
| 0x10   | RNG_TOTAL_BIT_COUNT_THRESHOLD | R/W    | Threshold / IRQ                            |
| 0x18   | RNG_INT_STATUS                | R/W1C  | Health: NIST fail, master lockout, …       |
| 0x1C   | RNG_INT_ENABLE                | R/W    | IRQ mask (unused in v1)                    |
| 0x20   | RNG_FIFO_DATA                 | R      | 32-bit random word                         |
| 0x24   | RNG_FIFO_COUNT                | R/W    | Low 8 bits = words available               |

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

## Power management (reset cause)

| Item      | Value                                                                         |
| --------- | ----------------------------------------------------------------------------- |
| Base      | `0xFE10_0000` (inside the mapped `peripherals` window — no window of its own) |
| `PM_RSTS` | `+0x20`, latched across a reset                                               |
| Access    | **read-only from this kernel**                                                |

Read once at boot and printed, which is why:

```
reset: PowerOn partition=0 (PM_RSTS=0x00001000)
```

`cpu::halt()` is `loop { wfe }` with IRQs masked and cannot exit, so a board
that boots again after `*** halt ***` was reset by something outside this
kernel. That happened during the 2026-08-06 session and three stories fit it —
a firmware watchdog never disarmed, a brownout, a glitch on the supply — with
nothing to choose between them. This register chooses.

| Bits                  | Cause     | Meaning                                                        |
| --------------------- | --------- | -------------------------------------------------------------- |
| `0x1000`              | `HADPOR`  | power-on                                                       |
| `0x0070`              | `HADWR*`  | watchdog (hard / full / request)                               |
| `0x0700`              | `HADSR*`  | software reset — nothing here writes one, so the firmware did  |
| `0x0007`              | `HADDR*`  | debug reset                                                    |
| `0..11` (interleaved) | partition | six two-bit fields, sharing the register with the causes above |

Decoded by `kernel_core::reset`, most specific first: a watchdog reset that
_also_ sets the power-on bit reads as a watchdog, because answering `PowerOn`
there would get the question wrong in the only direction that costs anything.
An empty register is `None` and not `PowerOn` — a block that latched nothing
must not be able to manufacture a clean power cycle.

QEMU `raspi4b` **does** model the block and reports a power-on. The first
version of this code assumed it did not, by analogy with RNG200, and the first
boot refuted it.

Read-only is structural rather than a convention: `PM_RSTC` and `PM_WDOG` sit
in the same block, reboot the board and arm the watchdog, and take a `0x5a`
password in the top byte — a write with the wrong value is a reset rather than
an error. `drivers::pm` has no write function to reach for by mistake.

## SD card

1. Partition 1: FAT32, bootable flag optional.
2. Files: see `docs/boot-chain.md`.
3. Deploy: `make blobs && make deploy SD_MOUNT=/path/to/boot`.
4. **Durable store partition (ADR-0066)**: a 1 MiB MBR partition of type
   `0x7f`, created once with `scripts/host/durable-partition.sh /dev/sdX`.
   The kernel discovers it from sector 0 by type — no fixed LBA anywhere —
   and keeps two CRC-committed slots inside it (header written last). A
   card without the entry boots with `durable-media: no-partition` and no
   media persistence. Read the slots from the host with
   `scripts/host/durable-read.sh /dev/sdX`.

The SD slot is EMMC2 on BCM2711 silicon; QEMU `raspi4b` wires its `-drive
if=sd` card to the legacy Arasan SDHCI instead, so the board bind probes
EMMC2 first and falls back, printing which host answered (`host=` on the
`durable-media:` line).

## Optional status display — Waveshare-class 3.5″ SPI TFT

**Status:** lab side-track **silicon-closed** for v1 status surface (not an
M-milestone). Policy: [ADR-0009](adr/0009-optional-spi-tft-debug-console.md)
(**accepted**), streaming CS [ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md).

Behind `--features debug-display` (`make FEATURES=debug-display img` / `deploy`):

- GPIO claim, `SpiBus` / `SpiDevice` / `with_bus`, polled SPI0
- ILI9486 PiScreen init + **regwidth-16** wire framing
- Boot: full-screen navy `HARBOR` + dirty-cell status text (8×8 slots, idle
  ticks/heap, panic banner)
- UART remains the full log (no serial mirror on glass)

**Trap:** `FEATURES` defaults to empty. A bare `make deploy` after a lab session
builds a headless image into the same `kernel8.img` and flashes it — the serial
log then has no `display:` line and the panel looks dead. Pass
`FEATURES=debug-display` on every glass deploy; `make img` also writes
`kernel8-debug-display.img` as a side copy when that feature is set. Oracle on
a healthy glass boot: `display: ILI9486 up  cdiv=…  bit_clk=… Hz  status`.

The image also says what it is, in the banner, before anything can go wrong:
`build: debug-display` or `build: headless (no SPI TFT, no bring-up gates)`. A
flashed card is otherwise indistinguishable from another one — `kernel8.img` is
whichever the last `make` invocation produced, and nothing in the file records
which. `make boot-check` asserts the banner against the behaviour in both
directions, so an image cannot claim the panel without bringing it up, nor
claim headless while touching it.

Evidence: [`verification.md`](verification.md#rng200-and-spi0-hardware). Touch
and higher SPI rates are open. Default (feature-off) images stay HAT-free.

This is **not** HDMI, DSI, or a VideoCore framebuffer. It is a GPIO HAT that
drives a TFT over **SPI0** with an **ILI9486** controller. Harbor talks to the
panel from EL1 through a generic SPI + panel stack; the HAT is a **BSP
profile**, not the architecture name.

### Target SKU

| Item                | Value                                                                                                                          |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| Family              | Waveshare 3.5inch RPi LCD (**A** / **B** / **C**) and pin-compatible clones (e.g. LCD wiki MPI3501)                            |
| Lab default profile | **Waveshare A** until a different PCB letter is confirmed in hand                                                              |
| Resolution          | 480×320                                                                                                                        |
| Colour              | RGB565 (65 536 colours)                                                                                                        |
| LCD controller      | ILI9486                                                                                                                        |
| Touch controller    | XPT2046 (phase 2 only — second `SpiDevice` on the same bus)                                                                    |
| Bus                 | SPI0, mode 0; **8 MHz** closed on silicon (raise toward 16–32 MHz only with glass re-check)                                    |
| SPI framing         | **regwidth=16 / buswidth=8** (fbtft `piscreen`): cmd/param as BE `u16` (`0x00,b`); pixels raw RGB565 — _not_ “16-bit SPI mode” |
| v1 paint model      | Dirty cells / window streams — **no** mandatory full-frame 300 KiB buffer                                                      |
| Product boot        | `HARBOR` fill + status text; colour bars are lab API only                                                                      |

**A / B / C:** same pin class for documentation; **not** drop-in for init or
MADCTL. Each letter is a named BSP profile with its own declarative init table.
Higher SPI rates are an optimisation after status text is closed (done).

### HAT pinout (physical 40-pin header numbers)

Source: Waveshare wiki for 3.5inch RPi LCD (A) and LCD wiki MPI3501 (same map).
Pins marked NC are unused by the HAT (UART on pins 8/10 remains free for the
serial console if you can reach the header edge or a pass-through).

| Phys             | BCM GPIO | Symbol           | Harbor use (v1)                                                          |
| ---------------- | -------- | ---------------- | ------------------------------------------------------------------------ |
| 1, 17            | 3V3      | 3.3 V            | Power                                                                    |
| 2, 4             | 5V       | 5 V              | Power                                                                    |
| 6, 9, 14, 20, 25 | GND      | Ground           | Common ground                                                            |
| 11               | **17**   | TP_IRQ           | Phase 2 (touch)                                                          |
| 18               | **24**   | LCD_RS           | DC: 0 = command, 1 = data                                                |
| 19               | **10**   | LCD_SI / TP_SI   | SPI0 MOSI (ALT0)                                                         |
| 21               | **9**    | TP_SO            | SPI0 MISO (touch; unused for LCD writes)                                 |
| 22               | **25**   | RST              | LCD hardware reset                                                       |
| 23               | **11**   | LCD_SCK / TP_SCK | SPI0 SCLK (ALT0)                                                         |
| 24               | **8**    | LCD_CS           | LCD chip select (active low; owned by `SpiDevice`, not the panel driver) |
| 26               | **7**    | TP_CS            | Phase 2 (touch CE1)                                                      |

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
3. ILI9486: reset, **datasheet-first** declarative init — a table of
   `InitOp::{Cmd, Data, DelayMs}` (`INIT_PISCREEN`), with MADCTL for
   landscape + BGR as one entry in it rather than a separate config type.
   Vendor trees (fbtft, Waveshare, TFT_eSPI) are cross-checks, not paste
   sources.
4. Timer-based delays (`CNTPCT`) for panel multi-ms waits — not CPU cycle spins.
5. Structured **status surface** (dirty cells); UART remains the full log stream.
6. Init **after** UART hello; `Result` on failure → one clear serial line, continue
   headless (never silent, never `unwrap` on the boot path).

**Rejected shortcuts:** bitbang SPI, monolithic vendor driver, full-frame blit
for status, CS toggled inside ILI9486, automatic serial mirror, paint from IRQ.

Touch (XPT2046) is phase 2 and requires the shared-bus `SpiDevice` shape above.

### References (external)

- ILI9486 datasheet (opcodes and timing floors — primary)
- Waveshare: [3.5inch RPi LCD (A)](https://www.waveshare.com/wiki/3.5inch_RPi_LCD_%28A%29)
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
