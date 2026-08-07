# Closed firmware policy

## Principle

Everything after the jump to `kernel8.img` is open source and built in this
repository. Stages before that jump are platform firmware owned by the
silicon/boot vendor. We **document, pin, and minimise** those stages. We do
not pretend they are open, and we do not spread closed code into the kernel.

## Inventory

| Component         | Location           | Controllable?     | Notes                       |
| ----------------- | ------------------ | ----------------- | --------------------------- |
| Boot ROM          | On-chip            | No                | Fused; starts the machine   |
| EEPROM bootloader | SPI flash on board | Update only       | Raspberry Pi Ltd binary     |
| `start4.elf`      | SD boot partition  | Yes (version pin) | VideoCore; loads our kernel |
| `fixup4.dat`      | SD boot partition  | Yes (version pin) | Companion to `start4.elf`   |
| `kernel8.img`     | SD boot partition  | Yes               | **Our code**                |

## What the kernel depends on the firmware for

Beyond loading `kernel8.img`, the pinned firmware determines machine state the
kernel inherits and does not reprogram from scratch. These are the reasons the
tag is a pin and not a preference:

| Inherited state                  | Effect if the firmware changes it                                                                                                                                                                                                                                                                                                                                           |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GIC security/group configuration | `drivers/gicv2.rs` drives the **Group 0 + `IAR`/`EOIR`** path, chosen because it is what worked on this firmware. In the Non-Secure view of GICv2, `GICD_CTLR` bit 0 is `EnableGrp1`, so this sequence depends on the state `start4.elf` leaves the distributor in. A firmware bump can stop interrupts being delivered — with no diagnostic beyond a silent `ticks=` line. |
| `CNTFRQ_EL0`                     | The arch timer rate is read, not set. Zero is reported as `TimerError::NoCounterFrequency`.                                                                                                                                                                                                                                                                                 |
| PL011 reference clock            | Assumed 48 MHz, which holds with `enable_uart=1` and `core_freq_min=500`. A different clock produces a console that prints garbage rather than nothing.                                                                                                                                                                                                                     |
| `CPACR_EL1`                      | Irrelevant by design: the kernel is softfloat and leaves FP trapping.                                                                                                                                                                                                                                                                                                       |

After bumping `firmware_tag`, boot once with `--features bringup` and check the
gates still pass; that is the cheapest way to catch a GIC regression. With tag
`1.20250430` they pass on a Pi 4B Rev 1.5 — `HPPIR=30`, `IAR=0x1e id=30` — so
the inherited configuration is confirmed for this pin, not merely assumed.

Integrity, as opposed to provenance, comes from `EXPECTED.sha256`: hashes
committed to this repository and checked before anything is installed. The
manifest records what was fetched; it agrees with itself whatever arrives.

## Operational rules

1. Blobs are **not** committed as opaque binary history without a manifest.
2. Fetch only via `scripts/host/fetch-blobs.sh` / `make blobs` at a **pinned tag**.
3. Integrity is `third_party/blobs/EXPECTED.sha256`, committed and verified
   before install. `MANIFEST.txt` records provenance — what was fetched, when —
   and is not a check: it is written from whatever arrived.
4. Bumping the firmware tag is a deliberate change: `ALLOW_UNVERIFIED=1` to
   fetch, review the printed sums, commit them to `EXPECTED.sha256` alongside
   the tag, re-flash, and re-run the bring-up gates.
5. Kernel code must not embed or require additional closed binaries.

## Why not “zero blobs”

- On Pi 4 there is no production path that reaches DRAM and the ARM cores
  without the EEPROM stage and VideoCore firmware.
- Projects such as `rpi-open-firmware` do not provide a usable Pi 4 stack.
- Replacing the board (e.g. fully open SoC) is out of scope; the target is
  Raspberry Pi 4 Model B.

Acceptance of EEPROM + `start4.elf` is therefore a **platform constraint**,
not a temporary workaround. The kernel boundary remains clean.
