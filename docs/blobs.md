# Closed firmware policy

## Principle

Everything after the jump to `kernel8.img` is open source and built in this
repository. Stages before that jump are platform firmware owned by the
silicon/boot vendor. We **document, pin, and minimise** those stages. We do
not pretend they are open, and we do not spread closed code into the kernel.

## Inventory

| Component | Location | Controllable? | Notes |
|-----------|----------|---------------|-------|
| Boot ROM | On-chip | No | Fused; starts the machine |
| EEPROM bootloader | SPI flash on board | Update only | Raspberry Pi Ltd binary |
| `start4.elf` | SD boot partition | Yes (version pin) | VideoCore; loads our kernel |
| `fixup4.dat` | SD boot partition | Yes (version pin) | Companion to `start4.elf` |
| `kernel8.img` | SD boot partition | Yes | **Our code** |

## Operational rules

1. Blobs are **not** committed as opaque binary history without a manifest.
2. Fetch only via `scripts/fetch-blobs.sh` / `make blobs` at a **pinned tag**.
3. `third_party/blobs/MANIFEST.txt` records tag, timestamp, and SHA-256.
4. Bumping the firmware tag is a deliberate change: update the script default,
   re-fetch, re-flash, and note the reason in the commit message.
5. Kernel code must not embed or require additional closed binaries for M0–M2.

## Why not “zero blobs”

- On Pi 4 there is no production path that reaches DRAM and the ARM cores
  without the EEPROM stage and VideoCore firmware.
- Projects such as `rpi-open-firmware` do not provide a usable Pi 4 stack.
- Replacing the board (e.g. fully open SoC) is out of scope; the target is
  Raspberry Pi 4 Model B.

Acceptance of EEPROM + `start4.elf` is therefore a **platform constraint**,
not a temporary workaround. The kernel boundary remains clean.
