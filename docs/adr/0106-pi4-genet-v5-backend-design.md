---
id: 0106
title: Pi 4 BCM2711 GENET v5 backend design
status: accepted
date: 2026-08-13
accepted: 2026-08-17
related: [0072, 0073, 0104, 0105]
---

# ADR-0106: Pi 4 BCM2711 GENET v5 backend design

## Status

**Accepted 2026-08-17** by the project owner, together with
[ADR-0105](0105-pi4-nic-backend-boundary.md). This is the board-specific design
that ADR-0105 requires: it selects the Pi 4B Ethernet device from the boot
description and defines the EL1-only backend boundary. Immutable under the ADR
lifecycle: change only via a successor ADR.

The design is now implemented and carries hardware evidence — probe, PHY
identify and BMSR classify, the Linux v5 init order with UniMAC taken out of
software reset, a bounded TX confirmed by UniMAC's own counter and by an
`0x88b5` frame on the wire, a bounded RX, recovery, and an absent-device
refusal. The road there, including the two defects that mattered and the
twenty-five single registers that did not, is recorded boot by boot in
[verification](../verification.md#hardware-evidence-pi-4-genet-v5-bring-up-2026-08-14--2026-08-17).

Acceptance covers the **backend**, not its publication: the network vocabulary
on `raspi4b` is still vacant, and binding it is the BSP composition step
ADR-0105 names.

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
After a successful identify it writes a bounded BMCR reset and prints
one `PhyInitReport` line (`genet: phy init (bmcr, not a nic)`), then
one `LinkReport` line (`genet: link=up|down (bmsr, not a nic)`) from
the post-reset probe sample.
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
on silicon. Silicon evidence lives in
[verification.md](../verification.md) (current stamp
`20260816-052739.log`, `src=3f2d01b8`: `phy init`, `tx unavailable (link down)`).
CONS retire is not a UniMAC send and not a wire frame. The network
vocabulary stays vacant.
A separate AArch64 control-plane slice in
`src/drivers/genet.rs` now validates that binding, performs a recoverable
revision probe, masks interrupts, stops both DMA engines, applies the
bounded UniMAC reset sequence, and can identify the DT PHY. `board-rpi4`
selects `Genet::probe`, `identify_phy`, `classify_link`,
`configure_queue0`, `enable_queue0` (after the frame pool exists),
`boot_after_program` (`GenetBoot`),
`submit_one_tx`, `submit_one_rx`, and `recover`. After Programmed the product writes TDMA rings 1–4 (32 BDs from 128)
and prints one `Rings14Report` line, then TDMA `ARB_CTRL=DMA_ARBITER_WRR`
(Linux `init_tx_queues`) and prints one `ArbiterReport` line, then
`DMA_PRIORITY_0/1/2` (`WrrPriority::V5`) and prints one `PriorityReport`
line. Enable writes
`RING_CFG`+`CTRL` only after Programmed; TDMA `RING_CFG` is
`TxRingSet` mask `V5_TX_RING_CFG` (`0x1f`, rings 0–4) and RDMA stays
the doorbell bit. `RingCfgReport` comes from that write.
`TxRingSet::tdma_ctrl` writes Linux `RING_BUF_EN` mask `0x1f`;
`rdma_ctrl` stays the doorbell bit. The product prints one
`RingBufReport` line from that write.
`submit_one_tx` writes UniMAC datapath (`CMD_SPEED_*`, `TX_EN`,
`RX_EN`, `PAD_EN`, `CRC_FWD`, `NO_LEN_CHK`), ORs `RGMII_LINK`,
then doorbells a TX BD with `APPEND_CRC` and the
v5 `QTAG`. `UMAC_TX_FLUSH` is pulsed at UniMAC init, not on each xmit. TX prints CONS retire (`tx cons`), not a send. After CONS
it prints a UniMAC TSV window (`0x49c` packed trap, Linux `0x4a8`
`tx_pkts`, `0x4ec` `pok`). Ring 16 TDMA `FLOW_PERIOD` carries
`MAX_FRAME << 16` (Linux does this for every ring except 0).
TBUF is programmed with `TBUF_64B_EN` and the probe carries a 64-byte
TSB prefix (`TX_DMA_BYTES=124`). `RBUF_TBUF_SIZE_CTRL` is written `1`
(Linux v3+ `init_umac`). `RBUF_CTRL` gets `RBUF_ALIGN_2B | RBUF_64B_EN`.
`RBUF_CHK_CTRL` gets `RXCHK_EN | L3_PARSE_DIS | SKIP_FCS` (`CRC_FWD`
is on) and prints one `RbufChkReport` line.
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
read the DT PHY, and run a bounded `init_phy`. `TxRingSet` separates
the TDMA mask from the doorbell queue; `QueueEnable` / `DmaPhase`
make enable a sequenced word (program, then `RING_CFG` + `CTRL`) rather
than a stray bit; cache invalidate-to-PoC is an AArch64 operation, not a
`qemu-virt` feature. The driver still does not select `board-rpi4` or
publish a network service.

### Unpaid silicon leftovers

Not a NIC claim. The unit is no longer one register: [ADR-0107](0107-genet-sequence-first-bring-up.md) makes it one coherent claim about the sequence, because twenty-five single variables did not move the outcome and the review found the defect in *where* they were written, not in *which*.

| Leftover | Linux fact | Harbor today |
| --- | --- | --- |
| `RING_BUF_EN` mask `0x1f` | `init_tx_queues` writes the same mask to `DMA_CTRL` `RING_BUF_EN` | Paid (HW) negative (`src=656be102`) |
| Program rings 1–4 | 32 BDs each after Q0's 128 | Paid (HW) negative (`src=7d1631b4`) |
| WRR priority words | `DMA_PRIORITY_0/1/2` weights | Paid (HW) negative (`src=414f4098`) |
| `init_phy` on the boot path | PHY setup before first xmit | Paid (HW) negative (`src=3f2d01b8`); submit now LinkDown |
| `RBUF_CHK_CTRL` | RX checksum control | Product writes `0x31`; **Paid (HW) negative** (`src=8981a0dc`) |
| **Init order** | `init_umac` and `hfb_init` run before `bcmgenet_init_dma`, which writes `DMA_EN` last (`bcmgenet.c:3351-3380`, `:3172-3180`) | Corrected 2026-08-17; **Paid (HW)** — negative alone (`src=8981a0dc`), part of the working sequence at `src=0a937a23` |
| **Flush settles** | `UMAC_TX_FLUSH` and the `RBUF_CTRL` latch pulsed inside `init_dma` with `udelay(10)`; `reset_umac` waits 10 µs then 2 µs (`:3113-3123`, `:2560-2571`) | Corrected 2026-08-17 (`CNTFRQ_EL0` waits, not readbacks); **Paid (HW)** (`src=0a937a23`) |
| **HFB** | `bcmgenet_hfb_clear` zeroes `HFB_CTRL`, both enable words, the eight index-to-ring words and all 48 filters, then enables filter 0 with length 4 — the default flow to ring 0 (`:720-741`) | Was never touched; corrected 2026-08-17; **Paid (HW)** (`src=0a937a23`) — `hfb cleared`, and RX completes |
| **TX/RX descriptor words** | `bcmgenet_xmit` sets no `DMA_OWN` and no `DMA_WRAP`; `bcmgenet_rx_refill` writes the address only (`:2184-2200`, `:2261`) | Corrected 2026-08-17; **Paid (HW) negative** (`src=8981a0dc`) |
| **Ring 0 geometry** | TX ring 0 owns 128 BDs, RX ring 0 owns 256, slot size `RX_BUF_LENGTH` on every ring (`:2730-2733`, `:3022`) | Was one BD sized by the frame, contradicting the rings 1–4 placed after it; corrected 2026-08-17; **Paid (HW) negative** (`src=8981a0dc`) |
| **RDMA `XON_XOFF_THRESH`** | Same per-ring offset TDMA calls `FLOW_PERIOD`; `bcmgenet_init_rx_ring` writes `(FC_THRESH_LO << 16) \| FC_THRESH_HI` (`:2817-2819`) | Was zero; corrected 2026-08-17; **Paid (HW) negative** (`src=8981a0dc`) |
| **UniMAC reset latch** | `bcmgenet_rbuf_ctrl_get/set` is `SYS_RBUF_FLUSH_CTRL` (SYS `0x08`), not `RBUF_CTRL`; `reset_umac` zeroes it and `bcmgenet_umac_reset` pulses `BIT(1)` in it to take the MAC out of reset before `init_umac` (`bcmgenet.c:127-140`, `:2563`, `:3299-3311`, `:3368`) | Was writing the wrong register and had no release pulse; corrected 2026-08-17; **Paid (HW)** (`src=9b074c54` refuted the latch itself, `src=4616443a` found the real one) |
| More than one TX BD | Linux posts the frame BDs it has | One BD |
| `TX_EN` settle | Datapath then transmit | Immediate doorbell |
| **`CMD_SW_RESET`** | `reset_umac` asserts it and stops; nothing in Linux clears it (`bcmgenet.c:2560-2571`), and `umac_enable_set` refuses to write `UMAC_CMD` while it is set (`:2540-2545`) | On BCM2711 it does **not** self-clear. Harbor polled for it, and the poll only ever passed because the write never landed. The 11:15 state dump read `cmd=0x1002067` — reset held, datapath written over it — for the whole boot. `reset()` now writes `UMAC_CMD = 0` after the settle; **Paid (HW)** (`src=4616443a`): first frame on the wire |

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
