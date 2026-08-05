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
script (`src/arch/aarch64/link.ld`) places `.text.boot` there.
`ENTRY(_start)` must remain the first executable symbol in the image.

## Exception level

With current platform firmware, the ARM cores typically enter the kernel in
**EL2**. `src/arch/aarch64/boot.s` detects `CurrentEL` and, when at EL2:

1. Programs `SCTLR_EL1` to its RES1 pattern (`0x30d00800`) with everything else
   clear — see below
2. Sets `HCR_EL2.RW` (EL1 is AArch64)
3. Allows EL1 physical timer/counter access via `CNTHCTL_EL2` (`EL1PCTEN|EL1PCEN`)
4. Clears `CNTVOFF_EL2`
5. Clears `CPTR_EL2.TFP`, so a stray FP instruction faults to the EL1 handler
   rather than to an EL2 with no vector table
6. `eret`s into EL1h with DAIF masked

All kernel code after that assumes **EL1**.

**EL3 is refused, not handled.** `CurrentEL` above 2 parks the core. Nothing in
this boot chain produces it — start4.elf enters at EL2 — but a custom armstub
can, and the EL2 sequence above would then program state that has no effect from
EL3 before `eret`ing into a configuration nobody set up. The code used to treat
"not EL2" as "already EL1", which is a wrong answer rather than a missing one.
There is no console that early, so the symptom is a silent park.

**Why `SCTLR_EL1` is not simply zeroed.** Bits 11, 20, 22, 23, 28 and 29 are
RES1 on ARMv8.0-A, the Cortex-A72's architecture level, and writing 0 to a RES1
field is UNPREDICTABLE. The reset value has them set; `msr sctlr_el1, xzr` took
them away, and `enable_translation` only read-modify-writes `M`/`C`/`I` on top,
so nothing restored them. Measured under QEMU the kernel ran with `SCTLR_EL1 =
0x1005`; it now reads `0x30d01805`. The board has not been re-measured since.

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
