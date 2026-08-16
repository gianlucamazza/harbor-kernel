---
id: 0106
title: Pi 4 BCM2711 GENET v5 backend design
status: proposed
date: 2026-08-13
related: [0072, 0073, 0104, 0105]
---

# ADR-0106: Pi 4 BCM2711 GENET v5 backend design

## Status

**Proposed.** This is the board-specific design required by ADR-0105. It
selects the actual Pi 4B Ethernet device from the boot description and defines
the EL1-only backend boundary. It does not claim implementation, emulation, or
hardware evidence.

## Evidence basis

The Raspberry Pi 4B device tree identifies the board as
`raspberrypi,4-model-b` / `brcm,bcm2711`, enables `genet`, selects
`phy-mode = "rgmii-rxid"`, and puts the PHY at MDIO address 1. The BCM2711
SoC description identifies the device as `brcm,bcm2711-genet-v5`, gives its
bus address and 64 KiB register window, declares two level-high GIC
interrupts, and includes the GENET MDIO child. These facts are from the
[Raspberry Pi 4B device tree](https://raw.githubusercontent.com/raspberrypi/linux/rpi-6.6.y/arch/arm/boot/dts/broadcom/bcm2711-rpi-4-b.dts)
and the [BCM2711 SoC device tree](https://raw.githubusercontent.com/raspberrypi/linux/rpi-6.6.y/arch/arm/boot/dts/broadcom/bcm2711.dtsi).

The [upstream GENET driver](https://raw.githubusercontent.com/torvalds/linux/master/drivers/net/ethernet/broadcom/genet/bcmgenet.c)
and its [register contract](https://raw.githubusercontent.com/torvalds/linux/master/drivers/net/ethernet/broadcom/genet/bcmgenet.h)
are reference material for register meanings, version-specific DMA layout,
interrupt separation, and reset sequencing. They are not copied into Harbor
and do not substitute for Harbor's host model or hardware capture.

## Context

The accepted P3 service ABI is transport-independent and already separates
EL0 packet policy from EL1 device ownership. The Pi 4B product path cannot
reuse the QEMU virtio transport: its onboard NIC is Broadcom GENET v5 behind
the BCM2711 SoC bus, with a RGMII PHY and a different descriptor and interrupt
model. A board conditional inside the virtio driver would make the transport
contract false and would leave reset, DMA addressability, cache ownership, and
PHY state unproved.

The current FDT reader deliberately stops at the closed discovery list and
does not parse `/soc`, `ranges`, `dma-ranges`, or device interrupt properties.
That is an explicit ADR-0072/0073 boundary, not permission to hardcode the
GENET address or interrupt numbers.

The first host-only slice now lives in `kernel_core::genet_fdt`. It extracts
the exact compatible GENET node, applies the ordered parent-bus mapping,
preserves the inherited interrupt-parent and two interrupt specifiers,
resolves the MDIO PHY phandle, and constrains DMA addresses from
`dma-ranges`. The product boot prints that extract as a `genet:` line
(`boot_report`): `binding ok … (fdt, not probed)` or `unavailable (…)`.
The line is not a `discover:` fact (ADR-0072/0073 keep `/soc` out of that
inventory), does not write MMIO, and does not bind the network vocabulary.
QEMU `raspi4b` deletes the GENET node from a loaded Pi 4 DTB (it logs the
compatible as disabled, then omits `/scb/ethernet`); the guest report is
therefore `unavailable (Missing)`, which is what that blob contains. The
unmodified fixture still extracts on the host. A real Pi firmware DTB keeps
the node enabled: stamp 2026-08-14, transcript `20260814-140651.log`
(`src=1aa3e894`) prints `genet: binding ok base=0xfd580000 len=0x10000
phy=rgmii-rxid (fdt, not probed)` and leaves the network vocabulary vacant.

The compiled BSP now maps that 64 KiB window (`GENET_BASE` in
`board-rpi4` `DEVICE_REGIONS`). When the FDT binding matches it, the
product calls `Genet::probe` (mask, stop DMA, UniMAC reset) and prints
the decoded revision. After a successful revision it also prints one
`PhyIdentify` line (`genet: phy=… (id, not a nic)` or a bounded refusal).
After a successful identify it also prints one `LinkReport` line
(`genet: link=up|down (bmsr, not a nic)`).
A Pi 4B (`src=30603cba`) printed
`genet: rev=6.0 patch=0x0 (mmio, not a nic)`,
`genet: phy=0x600d84a2 (id, not a nic)`,
`genet: link=down (bmsr, not a nic)`,
`genet: queue0 programmed (rings, not a nic)`,
`genet: queue0 enabled (dma, not a nic)`,
`genet: tx unavailable (link down)`,
`genet: rx unavailable (link down)`, and
`genet: reset recovered (idle, not a nic)`; encoded 6/7 are the v5
descriptor family (Linux remaps 6/7 → logical 5 and 5 → 4). The kernel does
not map a discovered PA (ADR-0072). After Enabled, one bounded TX and one
bounded RX are attempted only when BMSR is up; a down link refuses before
the doorbell or RX arm. Both refuse paths and the Idle recover are paid
on silicon. A cable-up product boot (`src=df9a0d80`,
`20260816-052739.log`) printed `queue0 programmed` / `enabled`,
`tbuf tsb`, `tbuf size`, `tx complete len=124`, and
`umac tsv packed=0 linux=0 pok=0` after a first `link=down` snapshot.
The Apple NIC pcap has no `0x88b5`. CONS posted the 124-byte TSB+probe
after `RBUF_TBUF_SIZE_CTRL=1`; neither the packed trap nor Linux
`tx_pkts`/`pok` counted a send. Serial complete is not a wire frame.
The network vocabulary stays vacant.
A separate AArch64 control-plane slice in
`src/drivers/genet.rs` now validates that binding, performs a recoverable
revision probe, masks interrupts, stops both DMA engines, applies the
bounded UniMAC reset sequence, and can identify the DT PHY. `board-rpi4`
selects `Genet::probe`, `identify_phy`, `classify_link`,
`configure_queue0`, `enable_queue0` (after the frame pool exists),
`submit_one_tx`, `submit_one_rx`, and `recover`. Enable writes
`RING_CFG`+`CTRL` only after Programmed. `submit_one_tx` writes UniMAC datapath (`CMD_SPEED_*`, `TX_EN`,
`RX_EN`, `PAD_EN`, `CRC_FWD`, `NO_LEN_CHK`), ORs `RGMII_LINK`,
then doorbells a TX BD with `APPEND_CRC` and the
v5 `QTAG`. `UMAC_TX_FLUSH` is pulsed at UniMAC init, not on each xmit. Complete is CONS posted (not OWN writeback). After CONS
it prints a UniMAC TSV window (`0x49c` packed trap, Linux `0x4a8`
`tx_pkts`, `0x4ec` `pok`). Ring 16 TDMA `FLOW_PERIOD` carries
`MAX_FRAME << 16` (Linux does this for every ring except 0).
TBUF is programmed with `TBUF_64B_EN` and the probe carries a 64-byte
TSB prefix (`TX_DMA_BYTES=124`). `RBUF_TBUF_SIZE_CTRL` is written `1`
(Linux v3+ `init_umac`). `RBUF_CTRL` gets `RBUF_ALIGN_2B | RBUF_64B_EN`.
Leftover `SYS_TBUF_FLUSH` is released. `UMAC_MIB_CTRL`
is pulsed then cleared so a zero TSV is not a stuck reset. Enabled+Up only;
`submit_one_rx` ORs `RX_EN` after the same speed word; unknown
autoneg is a refusal, not a silent 10 Mbps. After Enabled the product
also writes `SYS_PORT_CTRL=EXT_GPHY` and `EXT_RGMII_OOB_CTRL`
(`RGMII_MODE_EN`, `OOB_DISABLE` clear, `ID_MODE_DIS` clear for
`rgmii-rxid`) and prints one `RgmiiReport` line, then writes
`UMAC_MAX_FRAME_LEN` and station `UMAC_MAC0`/`UMAC_MAC1` and prints
one `UmacReport` line, then one `TbufReport` (`tbuf tsb`) after
setting `TBUF_64B_EN`. The product programs Linux v5 default TX
ring 0 (`DEFAULT_TX_RING`) for the bounded TX/RX doorbells and prints
one `Queue0Report` line. `recover` refuses Idle
and otherwise stops DMA, UniMAC-resets, and returns to Idle. It
does not publish a
network service and does not change this ADR's proposed status.

The companion `kernel_core::genet` model now also encodes and decodes the
v4+-style status word, keeps ownership and SOP/EOP/WRAP explicit, bounds ring
addresses against all discovered DMA apertures, and classifies both direct
and per-queue DMA interrupt bits. `RingState` exercises the ordered
driver/device/driver lifecycle and refuses full rings, missing completions,
bad status ownership, and malformed descriptors. The data-plane contract
adds `RingProgram` for queue 0 (v5 word-unit start/end after the 256-slot
descriptor RAM) and `MdioTxn` for a clause-22 read/write with busy/fail and
absent PHY-ID refusal. `PhyLink` classifies BMCR reset and BMSR link against
the binding's `rgmii-rxid` fact; it does not invent RGMII delay tables. This
remains a host contract. The AArch64 driver can write the queue-0 program,
read the DT PHY, and run a bounded `init_phy`. `QueueEnable` / `DmaPhase`
make enable a sequenced word (program, then `RING_CFG` + `CTRL`) rather
than a stray bit; cache invalidate-to-PoC is an AArch64 operation, not a
`qemu-virt` feature. The driver still does not select `board-rpi4` or
publish a network service.

Revision decoding and the v5 RDMA/TDMA offsets — descriptor RAM, then 17
rings, then the common control block — are also part of the pure contract,
so the MMIO layer will consume one tested register layout rather than
duplicate literal offsets.

The model keeps two address domains distinct: GENET v5 descriptor slots live
in the controller's internal RDMA/TDMA descriptor RAM, while the descriptor
address fields point at packet buffers constrained by the discovered DMA
apertures. A packet-buffer DMA window is therefore never reused as a ring
base.

## Decision

### 1. Discovery is device-tree-first

Before a backend can bind, the discovery layer must gain a separately tested
extraction contract for:

1. one enabled node with compatible `brcm,bcm2711-genet-v5` (a generic
   `brcm,genet-v5` match is not sufficient for the Pi4 product claim);
2. the parent bus `ranges` translation from the GENET bus address to the CPU
   physical address;
3. the exact register span (`0x10000` bytes);
4. both device interrupts, preserving the DT interrupt-parent and cell
   interpretation;
5. the GENET MDIO child and its PHY identity/address;
6. the applicable `dma-ranges` constraint for every ring, descriptor, and
   packet buffer allocation; and
7. `status = "okay"` and the board PHY mode.

The extraction must refuse malformed, absent, duplicated, ambiguous, or
incompatible nodes. It must report a bounded refusal and leave the network
vocabulary vacant. It must not select a device merely because a compiled
address happens to respond.

### 2. The backend owns GENET completely in EL1

The future backend owns the translated MMIO window, GENET v5 UMAC/RBUF/DMA
state, MDIO and PHY setup, descriptor memory, cache maintenance, both device
interrupts, link state, and reset/recovery. The EL0 service sees only the
existing directional token protocol and packet-pool grant from ADR-0104.

No GENET register, descriptor address, physical address, PHY address, link
status register, or DMA status is placed in a manifest or IPC message.

### 3. Use a separate transport implementation

The backend gets its own `kernel_core` model and `src/drivers`/BSP binding. It
does not add `cfg` branches to `virtio_mmio.rs` and does not reinterpret a
virtio split ring as a GENET ring.

The first implementation slice is intentionally one bounded TX path and one
bounded RX path. It may use only the minimum GENET ring/queue configuration
needed by that slice, but it must model the actual GENET v5 descriptor format:
the status block, address fields (including the v4+ high address word where
applicable), producer/consumer ownership, wrap/bounds checks, and the
version-specific ring register offsets. Any unsupported checksum, VLAN,
multi-queue, jumbo, power-management, or wake-on-LAN feature is refused or
disabled explicitly; it is not silently assumed.

### 4. Reset and interrupt sequencing are explicit

The backend must mask and clear both interrupt blocks before touching queue
state, stop RX/TX DMA before reclaiming descriptors, perform the documented
GENET UMAC software reset, and reinitialize the PHY/MDIO state before
advertising service readiness. IRQ0 handles link/MDIO events and IRQ1 handles
RX/TX queue events; handlers acknowledge and record bounded work only, while
the service path drains rings and performs copies.

There is no implicit reset success. A failed reset, invalid revision, link
initialization failure, stale generation, or malformed descriptor invalidates
the resident backend and makes the service refuse until a complete restart
has succeeded.

### 5. DMA and cache policy follows discovered limits

The allocator must prove that every GENET address is inside the device's
discovered DMA window and that descriptor/buffer arithmetic cannot overflow.
The backend uses explicit clean-before-device and invalidate-after-device
ownership transitions. It does not rely on the Pi's current cache-coherency
behavior, on a firmware-initialized mapping, or on the fact that an address
fits in the current board's RAM size.

## Required successor evidence

No implementation status changes until all of the following exist:

| Level | Required evidence |
| --- | --- |
| Host | Pure GENET v5 register/ring model tests: DT binding/refusal and address translation are covered by `kernel_core::genet_fdt`; DMA-window bounds, malformed descriptors, ownership transitions, interrupt classification, reset generations, and recovery refusal are covered by `kernel_core::genet`. |
| AArch64 compile | The board-agnostic GENET control-plane driver is linted for the freestanding AArch64 target; no hardware success is inferred from compilation. |
| Emulation | A GENET-capable emulator or deterministic model run labelled as non-hardware; QEMU `virtio-net` evidence cannot satisfy this row. |
| Hardware | Real Pi 4B serial capture with exact DTB/image/commit and board revision proving probe, GENET revision, PHY/link state, one bounded TX, one bounded RX, reset/recovery, and absent-device refusal. |

ADR-0105 remains the evidence gate. Until the successor implementation and
these captures exist, `board-rpi4` keeps no NIC backend and P3 remains
`done (QEMU)` only.

## Consequences

The QEMU virtio implementation remains portable and honest, while the Pi 4
backend gets a concrete device boundary rather than a compatibility shim. The
cost is an additional FDT extraction contract and a real GENET model before
hardware code can be called complete.

## Alternatives

| Option | Why not |
| --- | --- |
| Reuse the virtio backend with Pi4 conditionals | Different descriptor, interrupt, PHY, reset, and DMA contracts; it would hide unsupported assumptions. |
| Hardcode `0xfd580000` and GIC IDs | Bypasses DT verification and breaks the board-discovery and porting boundary. |
| Use the Linux driver as the implementation | Harbor is Linux-free; importing an OS driver would add ambient scheduling, memory, and networking assumptions. |
| Claim QEMU virtio as Pi4 evidence | The devices are different; ADR-0105 explicitly forbids that inference. |
