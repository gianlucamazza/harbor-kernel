# Completeness roadmap

**Single source of truth** for open and done **K** (kernel) and **P** (product)
tracks. Policy: [ADR-0026](adr/0026-kernel-and-product-completeness.md). Model
and layering: [architecture.md](architecture.md). Product narrative and use
cases: [vision.md](vision.md).

Status vocabulary: `open` | `in design` | `done (QEMU)` | `done (HW)`.

Order below is a **working plan** for product coherence — not a ship calendar.
**Design ADR before any boundary move** ([ADR-0001](adr/0001-multi-role-analysis.md)).

Foundation M0–M8 is **closed on Pi 4B**. Historical milestone narrative stays in
[foundation-history.md](foundation-history.md) — note that its **M/P** milestone
letters are the foundation-era vocabulary and are unrelated to the **K/P** tracks
below.

<a id="completeness-roadmap"></a>

---

## Mission (why this roadmap exists)

Quoted from its owner, [`vision.md`](vision.md) — `make doc-claims` compares the
two copies:

<!-- mission:begin -->

> Harbor is an OS where software arrives as **agents**, authority arrives as
> **grants**, and every boundary can be **shown to hold** — and the project
> **finishes** that OS, mechanism by mechanism and service by service.

<!-- mission:end -->

What each word buys, and why this file exists to track it:

| Pillar           | Meaning                                                                                                                     |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------- |
| **Agents**       | Software is isolation units (driver task + EL0 program), not ambient processes                                              |
| **Grants**       | Authority is slot-indexed and enumerable — nothing ambient                                                                  |
| **Evidence**     | Boundaries are shown (host / QEMU / HW), not only compiled                                                                  |
| **Completeness** | Kernel mechanisms **and** product services close under this model ([ADR-0026](adr/0026-kernel-and-product-completeness.md)) |

Harbor is **not** Linux/POSIX, not a perpetual lab, and not an LLM framework.
Product services (storage, net, UI, naming) live **as agents and compositions**
above a small TCB — same shape as the console endpoint.

---

## Horizons → product outcomes

Horizons are product stories ([vision](vision.md)); **status lives only in the
K/P tables** below. A horizon is “paid” when its listed tracks have evidence at
the named level — not when prose wishes it.

| Horizon                             | Product outcome                                                                                             | Tracks that close it                                                                                                                                                                                                                                                                                |
| ----------------------------------- | ----------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **H0 — Foundation**                 | Boundary lab on Pi 4B: tasks, caps, EL0, PL011 agent, blocking recv, console + beacon, cancel               | **Done (HW)** — M0–M8 + ADR-0024/0025                                                                                                                                                                                                                                                               |
| **H1 — Composition / appliance OS** | Multi-agent product you can compose and load without rebuild-only demos; early device/supervisor story      | **Paid (HW stamp 2026-08-08):** bar 1–7 + K5 thin + P2 durable + K4 budget + lifecycle residuals (serial `.serial-log/20260808-030219.log`). **Still open / residual:** Pi4 GENET v5 backend implementation/evidence for P3 and P4                                                                                 |
| **H2 — Boundary OS**                | Full boundary OS: fair execution, denser agents, production isolation, multi-core, remaining platform paths | **K4** + **K7** first + **K8** through steal + F-R1-P1 + **K5-S** Mini **done (HW)**; **K5** residual policy [ADR-0085](adr/0085-k5-density-residual-design.md) (**K5-H/B** later); **K7** residual [ADR-0084](adr/0084-k7-residual-policy.md); Pi4 GENET v5 backend implementation/evidence for P3 and P4 remain open; **P2** SD power-cycle **done (HW)** |

**H1 product bar (what “composition OS” means here):**

1. **Compose** — pack/inject/inspect agents + grants (P6; first slice done QEMU — host tools).
2. **Run several agents** from a product store, not a single beacon (P1; **done (HW)** — the current product store carries beacon, chirp, lookup, entropy and blob; stamp 2026-08-14, transcript `20260814-113438.log`, `loader: store n=5 image`. The 2026-08-11 stamp proves the earlier beacon + chirp composition on silicon; the 2026-08-08 stamp used builtin beacon+mute).
3. **Load without rebuild-only** — external store path (K6; **done (HW)** — same stamp shows both `loader: store n=2 image` and, on an earlier boot, the `loader: builtin` fallback); on-target put/get (P2; **done (HW)**); SD power-cycle **done (HW)** (ADR-0066, stamp 2026-08-09).
4. **Wait and drive devices as agents** — IRQ wait (K1; **done (HW)**); RNG map + IRQ-cap wait (K9; **done (HW)**).
5. **Supervise** — cancel + K2 auto-reap + park timeout + K10 reap/cascade (**done (HW)**).
6. **Move authority** — revoke + EL1 transfer + EL0 self/creator (**done (HW)**); peer via task-cap (**done (HW)** stamp 2026-08-09, ADR-0054; bands + lifecycle ADR-0055/0057).
7. **Find services** — EL1 registry + EL0 `SYS_RESOLVE` (**done (HW)**); resolve-grant (**done (HW)** stamp 2026-08-09, ADR-0052).

Use cases that pull H1: modular robot/industrial stacks, least-privilege edge
gateways, sealed composition firmware, on-device third-party sandbox
([vision § H1](vision.md#h1--composition--appliance-os)).

---

## K — microkernel mechanisms

| ID  | Track                                                     | Status                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Done when (sketch)                                                                     | Needs first                                                                              |
| --- | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| K1  | Wait-on-IRQ (first-class)                                 | **done (HW)** ([ADR-0028](adr/0028-wait-on-irq.md) + [ADR-0030](adr/0030-el0-irq-capability.md); Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | EL1 `wait_for_irq`; EL0 `SYS_WAIT_IRQ`; `irq-wait: woke` + `el0-irq: woke`             | ADR-0008 → 0028 → 0030                                                                   |
| K2  | Park reclaim (timeout and/or auto-reap on last send drop) | **done (HW)** ([ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md) + [ADR-0040](adr/0040-k2-park-timeout.md) + [ADR-0042](adr/0042-el0-recv-timeout.md); Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     | Last SEND hold drop; EL1/EL0 tick timeout → Cancelled                                  | 0025 → 0031 → 0040 → 0042                                                                |
| K3  | Cap transfer / revoke / endpoint release                  | **done (HW)** ([ADR-0032](adr/0032-k3-channel-revoke.md) + [ADR-0037](adr/0037-k3-cap-transfer.md) + [ADR-0041](adr/0041-el0-cap-transfer.md) + [ADR-0054](adr/0054-k3-peer-transfer-first-slice.md); Pi stamp 2026-08-09 covers revoke, transfer and peer/band/stale, `hw-transcript-check` clean)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Revoke; transfer; EL0 self/creator/peer; band + stale refusal oracles                  | 0017 → 0032 → 0037 → 0041 → 0053 → 0054 → 0055/0057                                      |
| K4  | Preemption or CPU budget                                  | budget **done (HW)** ([ADR-0046](adr/0046-k4-cooperative-cpu-budget.md); Pi stamp 2026-08-08); EL0 IRQ preemption **done (HW)** ([ADR-0064](adr/0064-k4-el0-preemption-first-slice.md); Pi stamp 2026-08-09); same-EL (EL1) preemption **done (HW)** ([ADR-0068](adr/0068-k4-el1-preemption-second-slice.md): frame-on-own-stack pivot; stamp 2026-08-09, transcript `20260809-151021.log`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Tick quantum + voluntary yield; any spinner, EL0 or EL1, loses CPU on the IRQ epilogue | 0006 → 0046 → 0051 → 0064 → 0068                                                         |
| K5  | Agent density (shrink/collapse driver half)               | **done (HW)** thin ([ADR-0044](adr/0044-k5-agent-density.md)); residual policy [ADR-0085](adr/0085-k5-density-residual-design.md); **K5-S** Mini **done (HW)** ([ADR-0086](adr/0086-k5-mini-stack-first-slice.md): stamp 2026-08-10); **K5-B design** **accepted** ([ADR-0089](adr/0089-k5-b-pair-collapse-design.md): no code until trigger); **K5-H** deferred; **K5-B code** deferred                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Thin + Mini HW; K5-B design paid; no `MAX_TASKS++` as density                          | 0023 → 0044 → 0085 → 0086 → 0089                                                         |
| K6  | External agent load + byte manifest                       | **done (HW)** ([ADR-0027](adr/0027-h1-external-agent-store.md) format, [ADR-0029](adr/0029-agent-store-in-image.md) placement; Pi stamp 2026-08-11, transcript `20260811-122821.log` — `loader: store n=2 image` on silicon loads and runs both agents, and an earlier boot in the same capture shows the `loader: builtin` fallback. The row said QEMU-only until 2026-08-11; the stamp taken for [ADR-0088](adr/0088-product-home-cpu.md) had already paid it)                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Image store inject; product prefers store, oracle empty → builtin                      | ADR-0021 → 0027 → 0029                                                                   |
| K7  | ASID (+ TTBR1 if required)                                | **done (HW)** first slice, stamp 2026-08-09 ([ADR-0047](adr/0047-k7-asid-isolation-design.md) + [ADR-0050](adr/0050-k7-asid-first-slice.md)); residual policy [ADR-0084](adr/0084-k7-residual-policy.md): **K7-M** switch-cost lab (optional), **K7-T** TTBR1 deferred-with-triggers, **K7-R** ASID rollover under pressure                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | First slice: ASID + dual-AS HW; TTBR1 only if trigger                                  | 0014 → 0047 → 0050 → 0084                                                                |
| K8  | SMP                                                       | **done (HW)** unpark ([ADR-0070](adr/0070-k8-smp-first-slice.md)); **done (HW)** IPI ([ADR-0074](adr/0074-k8-ipi-wake-second-slice.md)); **done (HW)** queues + shared-state ([ADR-0075](adr/0075-k8-per-core-queues-design.md)/[0076](adr/0076-k8-per-core-queues-first-slice.md)/[0077](adr/0077-smp-shared-state-discipline.md): stamp 2026-08-10); per-core timer+EL1 preempt **done (HW)** ([ADR-0078](adr/0078-k8-per-core-timer-preemption-design.md)/[0079](adr/0079-k8-per-core-timer-preemption-first-slice.md): stamp 2026-08-10); EL0-on-CPU1 **done (HW)** ([ADR-0080](adr/0080-k8-el0-on-cpu1-design.md)/[0081](adr/0081-k8-el0-on-cpu1-first-slice.md): stamp 2026-08-10); steal **done (HW)** ([ADR-0082](adr/0082-k8-work-stealing-design.md)/[0083](adr/0083-k8-work-stealing-first-slice.md): stamp 2026-08-10, `20260810-144305.log`); residual agent+TLB steal; parent [ADR-0048](adr/0048-k8-smp-design.md) | Unpark + IPI + queues + fair dual-core + steal HW; agent TLB residual                  | 0006 → 0048 → 0070 → 0074 → 0075 → 0076 → 0077 → 0078 → 0079 → 0080 → 0081 → 0082 → 0083 |
| K9  | Driver-as-agent beyond PL011 (+ IRQ caps)                 | **done (HW)** ([ADR-0034](adr/0034-k9-rng-driver-agent.md) + [ADR-0043](adr/0043-k9-irq-device-agent.md); Pi stamp 2026-08-08)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Map agent + IRQ-cap-only wait agent                                                    | 0013 → 0034 → 0043                                                                       |
| K10 | Supervisor lifecycle (restart, creator exit)              | **done (HW)** reap/cascade ([ADR-0033](adr/0033-k10-supervisor-reap.md) + [ADR-0038](adr/0038-k10-creator-exit-cascade.md); Pi stamp 2026-08-08); **force-exit Running** **done (HW)** ([ADR-0090](adr/0090-k10-force-exit-running.md); Pi stamp 2026-08-11, transcript `20260811-122821.log`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | `supervisor_reap_blocked` + `supervisor_force_exit`; exit cascades                     | 0018/0025 → 0033 → 0038 → 0090                                                           |

---

## P — product operating system

Product tracks deliver **services and platform paths** as agents/compositions,
not as a growing special-case syscall surface ([vision](vision.md) shape).

| ID  | Track                                   | Status                                                                                                                                                                                                                                                                                                                                                        | Done when (sketch)                                   | Typical deps       | Horizon      |
| --- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- | ------------------ | ------------ |
| P1  | Multi-agent product image beyond beacon | **done (HW)** (beacon + chirp + lookup + `entropy` + blob in store; first slice stamp 2026-08-11 `n=2`; [ADR-0101](adr/0101-composed-driver-agent.md) grew it to `n=3`, stamp 2026-08-13, transcript `20260813-101713.log` — `?H!R` on the wire when the board has RNG200; [ADR-0102](adr/0102-product-binds-a-name.md)/[0103](adr/0103-p2-el0-durable-endpoint.md) grew it to `n=5`, stamp 2026-08-14, transcript `20260814-113438.log` — `N` and `S` on the wire) | Product store n≥2; agents run via console endpoint   | ADR-0027/0029      | H1           |
| P2  | Storage path (block + load/persist)     | **done (HW)** first slices ([ADR-0036](adr/0036-p2-keyed-blob-store.md) + [ADR-0045](adr/0045-p2-durable-store.md); Pi stamp 2026-08-08); SD power-cycle **done (HW)** ([ADR-0066](adr/0066-sd-media-durable-store.md): EMMC2 PIO, 0x7f partition, A/B slots; Pi stamp 2026-08-09, transcripts `20260809-140657/140804.log` + host reader); EL0 request/reply endpoint **done (HW)** ([ADR-0103](adr/0103-p2-el0-durable-endpoint.md); stamp 2026-08-14, transcript `20260814-113438.log` — `blob: put ok`, `blob: got`, `S`) | Put/get + durable section + media across power cycle | 0036 → 0045 → 0066 → 0103 | H1 → H2      |
| P3  | Network agent + caps                    | **transport + split-ring descriptor lifecycle + EL1 packet service + directional caps integrated** — host-tested modern feature negotiation, descriptor arithmetic, bounded packet ownership, wire-token validation, QEMU `virt` evidence for private RX/TX buffers, `DRIVER_OK`, TX descriptor submission/completion, copy-backed TX acceptance/completion through directional endpoints, retained frame ownership, 32 slot IRQ bindings, deterministic peer RX payload delivery, service reset/recovery, and absent-device refusal ([ADR-0104](adr/0104-p3-edge-network-composition.md)); Pi 4 GENET v5 backend implementation and NIC hardware evidence remain open under [ADR-0105](adr/0105-pi4-nic-backend-boundary.md)                                                                                                                                                                                                 | Network I/O only via granted caps                    | K1/K9 helpful      | H1 edge → H2 |
| P4  | Display/input product path              | **open** — deferred ([ADR-0049](adr/0049-deferred-residuals.md): no composition target). The lab panel is **retired** ([ADR-0094](adr/0094-retire-debug-display.md)); P4 starts from a composition, not from that driver                                                                                                                                      | A product path, when a composition names one         | Device agents      | H1 UI → H2   |
| P5  | Naming / discovery / system services    | **done (HW)** ([ADR-0035](adr/0035-p5-name-registry.md) + [ADR-0039](adr/0039-p5-el0-resolve.md) + [ADR-0052](adr/0052-p5-resolve-grant.md); Pi stamp 2026-08-09 covers bind/resolve and non-ambient grant, `resolve-grant: refused` on silicon)                                                                                                              | Bind/resolve + non-ambient grant                     | 0035 → 0039 → 0052 | H1 → H2      |
| P6  | Compose/audit tooling                   | **done (QEMU)** first slice (pack / inject / inspect)                                                                                                                                                                                                                                                                                                         | Host tools for store composition and audit           | P1                 | H1           |

---

## Next working order (post H1 HW stamp)

Priority is **mission fit**, not ID order. ADR before boundary code.

| Order | Track(s)                                                  | Why now                                                                                                                                                                                                                                                                                                                                                                                                                 | Tracker                                                         |
| ----: | --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
|     — | ~~Product-boot-check / oracle census~~                    | **Paid** — composition-minimum + `oracle-census`                                                                                                                                                                                                                                                                                                                                                                        | —                                                               |
|     — | ~~Product multi-core policy~~ (`home_cpu`)                | **Paid (HW)** — [ADR-0088](adr/0088-product-home-cpu.md), stamp 2026-08-11                                                                                                                                                                                                                                                                                                                                              | —                                                               |
|     — | ~~**K5-B** design ADR~~                                   | **Paid (design)** — [ADR-0089](adr/0089-k5-b-pair-collapse-design.md); code only if trigger                                                                                                                                                                                                                                                                                                                             | —                                                               |
|     — | ~~**K10** force-exit Running~~                            | **Paid (HW)** — [ADR-0090](adr/0090-k10-force-exit-running.md), stamp 2026-08-11                                                                                                                                                                                                                                                                                                                                        | —                                                               |
|     — | ~~Slot meter measured~~                                   | **Paid (HW)** — [ADR-0098](adr/0098-slot-meter-measured.md); `oracle-census` boots the product and reads `slots=`. Measured product peak **8** on QEMU (`entropy` refused — no RNG200) and **9** on a Pi 4B that runs all five store agents (`20260814-113438.log`, `slots=4/9`); `MAX_TASKS` is **57**                                                                                                                                                                                                                                                                             | —                                                               |
|     — | ~~Composition vocabulary~~                                | **Paid (HW)** — [ADR-0099](adr/0099-composition-vocabulary.md), stamp 2026-08-12; `held` is declared, a failed mint leaves a hole instead of shifting every later index, `make vocabulary-sync` compares the kernel's copy against the packer's                                                                                                                                                                         | —                                                               |
|     — | ~~Device windows~~                                        | **Paid (HW)** — [ADR-0100](adr/0100-device-windows.md), stamp 2026-08-13; a window is named by **index** into a declared vocabulary, never by a physical address on the wire, so a store cannot mint memory. The 0100 slice shipped the empty vocabulary (refusal by `index >= 0`); [ADR-0101](adr/0101-composed-driver-agent.md) then declared `rng` at 0                                                                 | —                                                               |
|     — | ~~Composed driver-agent~~                                 | **Paid (HW)** — [ADR-0101](adr/0101-composed-driver-agent.md), stamp 2026-08-13; `entropy` arrives in the store holding a window instead of being compiled in, and reads `RNG_CTRL` before it speaks, so an encoder that dropped the load changes the byte on the wire. QEMU's `raspi4b` has no RNG200, so the refusal path is what runs daily and the reading — `R` on the wire from a Pi 4B's RNG200 — is the stamp's | —                                                               |
|     — | ~~**Services on endpoints** (name bind)~~                 | **Paid (HW)** — [ADR-0102](adr/0102-product-binds-a-name.md); the product binds `console`, `lookup` arrives without the slot and finds it by `SYS_RESOLVE`, `N` is on the wire every `product-boot-check`. Stamp 2026-08-14, transcript `20260814-113438.log`. The console *server* stays EL1 (M8)                                                                                          | —                                                               |
|     — | ~~GENET FDT report~~                                  | **Paid (HW)** — stamp 2026-08-14, transcript `20260814-140651.log` (`src=1aa3e894`): `genet: binding ok base=0xfd580000 len=0x10000 phy=rgmii-rxid (fdt, not probed)` and `authority: network vocabulary VACANT`. QEMU remains `unavailable (no dtb)` without `-dtb` and `unavailable (Missing)` with the fixture (QEMU deletes the node). Not a NIC; ADR-0105/0106 stay proposed. | — |
|     — | ~~GENET compiled window + `Genet::probe`~~            | **Paid (HW)** — last boot in `20260814-140651.log` (`src=61fe6774`): `binding ok` and `genet: rev=6.0 patch=0x0 (mmio, not a nic)`, vocab vacant. Encoded 6/7 are v5 (Linux remaps those to logical v5; encoded 5 is v4). QEMU stays `probe unavailable (no binding)`. Not a NIC; ADR-0105/0106 stay proposed. | — |
|     — | ~~GENET PHY identify~~                                | **Paid (HW)** — stamp 2026-08-14, transcript `20260814-174803.log` (`src=58b7448c`): `genet: phy=0x600d84a2 (id, not a nic)` after `rev=6.0`, vocab vacant. QEMU has no successful probe, so no PHY line. Not a NIC; ADR-0105/0106 stay proposed. | — |
|     — | ~~GENET link classify~~                               | **Paid (HW)** — stamp 2026-08-14, last boot in `20260814-174803.log` (`src=ee650e06`): `genet: link=down (bmsr, not a nic)` after `phy=0x600d84a2`. Cable-down is down, not a failure. QEMU has no successful probe, so no link line. Not a NIC; ADR-0105/0106 stay proposed. | — |
|     — | ~~GENET queue-0 program~~                             | **Paid (HW)** — stamp 2026-08-14, last boot in `20260814-174803.log` (`src=f7404550`): `genet: queue0 programmed (rings, not a nic)` after `frames:` (live `frames_free=510`, two held). DMA stays disabled. QEMU has no successful probe, so no queue0 line. Not a NIC; ADR-0105/0106 stay proposed. | — |
|     — | ~~GENET queue-0 enable~~                              | **Paid (HW)** — stamp 2026-08-14, last boot in `20260814-174803.log` (`src=0b1dc0b9`): `genet: queue0 enabled (dma, not a nic)` after `queue0 programmed`. No TX/RX completion. QEMU has no successful probe, so no enable line. Not a NIC; ADR-0105/0106 stay proposed. | — |
|     — | ~~GENET bounded TX (refuse)~~                         | **Paid (HW)** — stamp 2026-08-14, last boot in `20260814-174803.log` (`src=34b3d132`, PowerOn): `genet: tx unavailable (link down)` after `queue0 enabled`. Cable-down refuses before the doorbell; not a completed TX. QEMU has no successful probe, so no TX line. Not a NIC; ADR-0105/0106 stay proposed. | — |
|     — | ~~GENET bounded RX (refuse)~~                         | **Paid (HW)** — stamp 2026-08-14, last boot in `20260814-174803.log` (`src=9cd383eb`, PowerOn): `genet: rx unavailable (link down)` after the TX refuse. Cable-down refuses before UniMAC `RX_EN`; not a completed RX. QEMU has no successful probe, so no RX line. Not a NIC; ADR-0105/0106 stay proposed. | — |
|     — | ~~GENET reset/recovery~~                              | **Paid (HW)** — stamp 2026-08-14, last boot in `20260814-174803.log` (`src=30603cba`, PowerOn): `genet: reset recovered (idle, not a nic)` after the TX/RX refuses. Not a NIC; ADR-0105/0106 stay proposed. QEMU has no successful probe, so no reset line. | — |
|     — | ~~GENET TX/RX timeout (cable)~~                       | **Paid (HW)** — stamp 2026-08-14, transcript `20260814-232303.log` (`src=30603cba`, PowerOn): first BMSR `link=down`, then `tx unavailable (timeout)` / `rx unavailable (timeout)` ~90 µs after enable. Laptop Apple RX=0. Not a completed frame. Not a NIC; ADR-0105/0106 stay proposed. | — |
|     — | ~~GENET honest CONS~~                                 | **Paid (host)** — `cons_is_idle` refuses junk CONS before the doorbell; complete needs CONS posted and Driver OWN. Silicon `src=7a2b7ab2` still printed `timeout` (CONS was idle). Not a NIC. | — |
|  next | **GENET UniMAC speed**                                | On Enabled+Up, `classify_aneg_speed` (clause-22 LPA + CTRL1000/STAT1000) writes UniMAC `CMD_SPEED_*` before `TX_EN`. Unknown/no-ANEG = `unknown speed`, not 10 Mbps. Host-tested. QEMU has no successful probe. Silicon unpaid. Not a NIC; ADR-0105/0106 stay proposed. | — |
|  held | **K5-H** design (**no slot wall**)                        | The trigger now has a number instead of an opinion: measured product peak **8 of 57** slots on QEMU (6 live + 2 idle; `entropy` refused — no RNG200) and **9 of 57** on a Pi 4B that runs the five-agent store (`slots=4/9` after the agents exit). ADR-0085 §3 keeps K5-H deferred until that peak approaches the ceiling; `oracle-census` (QEMU) is what will say so                                                                                                    | —                                                               |
| watch | **K7-M** switch-cost lab / **K7-T** if trigger            | Optional; policy [ADR-0084](adr/0084-k7-residual-policy.md)                                                                                                                                                                                                                                                                                                                                                             | [#21](https://github.com/gianlucamazza/harbor-kernel/issues/21) |
| gated | **K5-H** / agent+TLB / **H3 L1+** / **P4** / cores 2–3 | Trigger or composition target only; P3 QEMU implementation is complete, while the Pi4 backend remains evidence-gated by ADR-0105                                                                                                                                                                                                                                                                                                                                                      | —                                                               |

**H1 entry + depth first slices are paid (HW stamp 2026-08-08).**  
K7 ASID slice **done (HW)** (stamp 2026-08-09). Resolve-grant + peer transfer
**done (HW)** (same stamp). K4 EL0 + EL1 preemption **done (HW)** (ADR-0064/0068).
K8 unpark + IPI + queues first slice **done (HW)** (ADR-0070/0074/0076/0077;
stamp 2026-08-10, transcript `20260810-130305.log`: `smp: core1 alive` +
`ipi` + `ran`). P3 is **done (QEMU)** at the accepted ADR-0104 target;
the Pi 4 path prints a `genet:` FDT report, a `rev=6.0` MMIO probe, PHY/link/queue0 lines, and leaves the network vocabulary vacant;
Pi4 GENET v5 backend implementation and NIC evidence remain open under ADR-0105,
and P4 remains deferred. P2
power-cycle **done (HW)** (2026-08-09).
**H3 L0** **done (QEMU-x86)** ([ADR-0071](adr/0071-h3-l0-x86-qemu-first-slice.md)).
**Discovery** **done (QEMU)** + **done (HW)** (ADR-0072/0073). **K8 per-core
timer + EL1 preempt on CPU 1** **done (HW)**
([ADR-0078](adr/0078-k8-per-core-timer-preemption-design.md)/[0079](adr/0079-k8-per-core-timer-preemption-first-slice.md);
stamp 2026-08-10, transcript `20260810-132749.log`). **EL0-on-CPU1**
**done (HW)**
([ADR-0080](adr/0080-k8-el0-on-cpu1-design.md)/[0081](adr/0081-k8-el0-on-cpu1-first-slice.md);
stamp 2026-08-10, transcript `20260810-134826.log`: `preempt-el0-cpu1: rotated`

- `spinner exited`). **K8 steal** **done (HW)**
  ([ADR-0082](adr/0082-k8-work-stealing-design.md)/[0083](adr/0083-k8-work-stealing-first-slice.md);
  stamp 2026-08-10, transcript `20260810-144305.log`: `smp: steal ok`).
  **K7 residual policy** paid ([ADR-0084](adr/0084-k7-residual-policy.md):
  option C current; K7-M optional lab; K7-T trigger-gated; K7-R under pressure).
  **K5 residual policy** paid ([ADR-0085](adr/0085-k5-density-residual-design.md)).
  **K5-S Mini** **done (HW)** ([ADR-0086](adr/0086-k5-mini-stack-first-slice.md),
  stamp 2026-08-10). F-R1-P1 shared-state **done (HW)**; **loader tables** locked
  (0077 amended 2026-08-11), then every shared table restated as `sync::Mutex<T>`
  ([ADR-0091](adr/0091-data-in-lock.md)). Product evidence hygiene **paid**.
  **Product multi-core `home_cpu` paid (HW)** ([ADR-0088](adr/0088-product-home-cpu.md), stamp 2026-08-11).
  **K5-B design paid** ([ADR-0089](adr/0089-k5-b-pair-collapse-design.md); code deferred).
  **K10 force-exit paid (HW)** ([ADR-0090](adr/0090-k10-force-exit-running.md), stamp 2026-08-11).
  **Composition vocabulary + first composed driver-agent paid (HW)**
  ([ADR-0099](adr/0099-composition-vocabulary.md)/[0100](adr/0100-device-windows.md)/[0101](adr/0101-composed-driver-agent.md),
  stamps 2026-08-12…13).
  **Product name bind + P2 EL0 durable endpoint paid (HW)**
  ([ADR-0102](adr/0102-product-binds-a-name.md)/[0103](adr/0103-p2-el0-durable-endpoint.md),
  stamp 2026-08-14, transcript `20260814-113438.log`).
**Next:** Pi4 GENET v5 backend implementation and NIC evidence for the completed
QEMU P3 edge-gateway target. The `genet:` FDT report is **done (HW)**
(`20260814-140651.log`) and is not that gate.
Held: K5-H until the slot peak approaches 57. Trigger-only: K5-B **code**,
K7-T, agent+TLB, H3 L1+, P4.

```text
Mission: agents · grants · evidence · finish the OS
                │
    H0 foundation ████████ done (HW)
                │
    H1 composition ████████ done (HW) stamp 2026-08-08
                │
    H2 boundary    ████████ K4+K7+K8+F-R1+K5-S HW; residual K5-H·B / K7-T if trigger
                │
    H3 host-class  █░░░░░░░ L0 done (QEMU-x86); L1+ open
```

---

## Standing watches (not completeness tracks)

| Work                          | Done when                                                                                                                                                                                                                                                           | Issue                                                           |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| ~~**ADR-0020 expiry watch**~~ | **Closed 2026-08-11** — the trait went with the panel ([ADR-0094](adr/0094-retire-debug-display.md)). A permanent watch is a retirement nobody scheduled                                                                                                            | [#14](https://github.com/gianlucamazza/harbor-kernel/issues/14) |
| **Lab second ISA (x86)**      | L0 **done (QEMU-x86)** ([ADR-0071](adr/0071-h3-l0-x86-qemu-first-slice.md)); intent [ADR-0067](adr/0067-host-lab-second-isa-intent.md); matrix in [design/host-lab-platform-matrix.md](design/host-lab-platform-matrix.md). Not a K/P track; does not reorder Pi H2 | —                                                               |
| **Host-class north star**     | [ADR-0069](adr/0069-harbor-host-class-north-star.md) + vision **H3**: native Harbor as primary OS (L0–L4). Name stays **Harbor**. QEMU lab L0 paid; L1+ separate ADRs                                                                                               | —                                                               |

---

## Out of model (permanent non-goals)

These are **not** completeness tracks ([ADR-0026](adr/0026-kernel-and-product-completeness.md)):

- Linux / POSIX / glibc compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
- Multi-tenant cloud hypervisor (unless a future ADR owns it)
- Being an AI agent framework (may _host_ workers; is not a chat SDK)

---

## How to extend

1. Add a row (or split a slice) in this file first; if it is product-facing,
   say which horizon and which use case it serves.
2. Write/accept a design ADR before boundary code.
3. Land code + gates; flip status only with evidence named in the row.
4. Point any GitHub tracker at **this file** — do not invent a second status
   table. When a track is paid, **close or refresh** the issue in the same
   pass (stale open issues are a second, lying dashboard).
5. Keep [vision.md](vision.md) narrative aligned when a horizon’s “paid / still
   pay” list changes (vision may lag a day; **status** never leaves this file).
