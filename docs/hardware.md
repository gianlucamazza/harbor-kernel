# Hardware — Raspberry Pi 4 Model B

## Target

| Item                 | Value                                         |
| -------------------- | --------------------------------------------- |
| Board                | Raspberry Pi 4 Model B                        |
| SoC                  | BCM2711                                       |
| Cores                | 4× Cortex-A72 (only core 0 active through M1) |
| Arch                 | AArch64, EL1 after bootstrap                  |
| Peripheral MMIO base | `0xFE00_0000`                                 |

## AArch64 QEMU `virt` P3 composition

The network transport uses a separate board bind, selected with
`board-qemu-virt`; it is not a Raspberry Pi hardware claim and is not part of
the default product image. The reproducible check is:

```bash
make qemu-virtio-check
```

The check boots `virt` with GICv2, a 128 MiB RAM aperture at `0x4000_0000`,
PL011 at `0x0900_0000`, and QEMU's virtio-mmio slot aperture at
`0x0a00_0000`. It runs once with `virtio-net-device` and once without it. The
current evidence proves DTB reservation/mapping, modern `VERSION_1` transport
negotiation, two size-8 split queues backed by six EL1 ring pages, private EL1
RX/TX buffers, `DRIVER_OK`, split-ring TX descriptor submission/completion,
retained EL1 ownership, 32 slot IRQ bindings, deterministic peer RX payload
delivery, the service reset/recovery boundary, and absent-device refusal. The
QEMU-only built-in edge-gateway capability path is not a Pi4 hardware claim;
the separate Pi4 backend evidence gate is [ADR-0105](adr/0105-pi4-nic-backend-boundary.md).

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

## Retired: the SPI TFT status surface

A Waveshare-class 3.5″ ILI9486 HAT was brought up on this board and closed on
silicon in August 2026 — SPI0 pinmux, polled transfers, regwidth-16 wire
framing, panel init and a status grid, all behind a `debug-display` feature.
[ADR-0094](adr/0094-retire-debug-display.md) **retired it** on 2026-08-11: it
compiled in every `make check` and was executed by nothing, and no product
composition named a panel.

Nothing in this tree drives a display today. The board still has the pins; the
kernel no longer knows about them.

Where the detail went, if a panel ever comes back:

- the decisions and what they cost — [ADR-0009](adr/0009-optional-spi-tft-debug-console.md)
  and [ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md) (both **superseded**),
  including the regwidth-16 SKU note and the measured 8 MHz bit-clock ceiling;
- the pinout, the bring-up sequence and the silicon transcripts —
  [`verification.md`](verification.md), in the dated 2026-08-05 evidence
  sections, which are records and stay as they are;
- the reusable half — `kernel_core::{display, textgrid, font8x8, spi}` — is
  still in the tree, pure and host-tested. What went is the binding to one HAT,
  which would have to be rewritten against the SPI and DMA facts of the day
  anyway.

## Safety

- Power off before reseating the SD card or UART leads.
- Do not drive GPIO from 5 V.
- Secondary cores are parked in `wfe`; do not assume SMP readiness.
