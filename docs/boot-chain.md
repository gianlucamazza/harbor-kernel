# Boot chain — Raspberry Pi 4 Model B

## Stages

```
Power-on
   │
   ▼
On-chip Boot ROM          (immutable, closed)
   │
   ▼
SPI EEPROM bootloader     (closed, updatable via rpi-eeprom)
   │  reads FAT boot partition
   ▼
start4.elf + fixup4.dat   (VideoCore firmware, closed — see docs/blobs.md)
   │  DRAM training, clocks, loads kernel
   ▼
kernel8.img @ 0x80000     (this project — fully open)
   │
   ▼
_start (EL2 or EL1) → drop to EL1h if needed → kernel_main
```

## Load address

AArch64 kernels are loaded at physical address **`0x80000`**. The linker
script (`link.ld`) places `.text.boot` there. `ENTRY(_start)` must remain the
first executable symbol in the image.

## Exception level

With current platform firmware, the ARM cores typically enter the kernel in
**EL2**. `src/boot.s` detects `CurrentEL` and, when at EL2:

1. Sets `HCR_EL2.RW` (EL1 is AArch64)
2. Allows EL1 physical timer/counter access via `CNTHCTL_EL2` (`EL1PCTEN|EL1PCEN`)
3. Clears `CNTVOFF_EL2`
4. `eret`s into EL1h with DAIF masked

All kernel code after that assumes **EL1**.

## Boot partition contents

| File | Source | Required |
|------|--------|----------|
| `kernel8.img` | `make` / this repo | yes |
| `config.txt` | `boot/config.txt` | yes |
| `start4.elf` | `make blobs` | yes |
| `fixup4.dat` | `make blobs` | yes |
| DTB | optional | not required for M0/M1 |

## `config.txt` keys we rely on

| Key | Purpose |
|-----|---------|
| `arm_64bit=1` | AArch64 kernel |
| `kernel=kernel8.img` | Image name |
| `enable_uart=1` | PL011 clock enable via platform firmware |
| `enable_gic=1` | Use GIC-400 (not legacy ARMC) |
| `core_freq_min=500` | Stable core clock so UART clock stays 48 MHz |

GPIO 14/15 ALT0 is programmed by the kernel BSP after entry. No device tree
or overlays are part of the M0/M1 boot contract.

## What we do not use

- Linux kernel or initramfs
- Device tree / overlays (M0/M1)
- U-Boot (optional later as an open intermediate stage; does not remove EEPROM/`start4.elf`)
- Open VideoCore replacements (not viable for Pi 4 at this time)
