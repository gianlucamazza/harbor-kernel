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

## Safety

- Power off before reseating the SD card or UART leads.
- Do not drive GPIO from 5 V.
- Secondary cores are parked in `wfe`; do not assume SMP readiness.
