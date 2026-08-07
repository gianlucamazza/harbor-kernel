# Completeness roadmap

**Single source of truth** for open and done **K** (kernel) and **P** (product)
tracks. Policy: [ADR-0026](adr/0026-kernel-and-product-completeness.md). Model
and layering: [architecture.md](architecture.md). Product narrative:
[vision.md](vision.md).

Status vocabulary: `open` | `in design` | `done (QEMU)` | `done (HW)`.

Order is a working plan — **design ADR before any boundary move**
([ADR-0001](adr/0001-multi-role-analysis.md)).

Foundation M0–M8 is **closed on Pi 4B**. Historical milestone narrative stays in
[architecture § roadmap](architecture.md#roadmap).

<a id="completeness-roadmap"></a>

## K — microkernel mechanisms

| ID | Track | Status | Done when (sketch) | Needs first |
| --- | --- | --- | --- | --- |
| K1 | Wait-on-IRQ (first-class) | **done (QEMU)** first slice ([ADR-0028](adr/0028-wait-on-irq.md)); EL0 IRQ cap open | EL1 `wait_for_irq(cookie)`; timer/UART `signal`; oracle `irq-wait: woke` | ADR-0008 → 0028; EL0 syscall successor |
| K2 | Park reclaim (timeout and/or auto-reap on last send drop) | **open** | Orphan parks do not hold `MAX_TASKS` forever without policy | Successor to ADR-0025 |
| K3 | Cap transfer / revoke / endpoint release | **open** | Authority can move and die without reboot; stale generation exercised by real release | ADR-0017 successor |
| K4 | Preemption or CPU budget | **open** | Hostile busy-loop is not permanent DoS residual | Successor to ADR-0006; name agent-pair impact (0023) |
| K5 | Agent density (shrink/collapse driver half) | **open** | Many small agents without 16 KiB kernel stack each by default | Successor to ADR-0023 |
| K6 | External agent load + byte manifest | **done (QEMU)** ([ADR-0027](adr/0027-h1-external-agent-store.md) format, [ADR-0029](adr/0029-agent-store-in-image.md) placement) | Image store inject; product prefers store, oracle empty → builtin | ADR-0021 → 0027 → 0029 |
| K7 | ASID (+ TTBR1 if required) | **open** | Production isolation without cloned-kernel-only story as the end state | Design ADR |
| K8 | SMP | **open** | Multi-core runqueue/IRQ model on silicon | Design ADR |
| K9 | Driver-as-agent beyond PL011 (+ IRQ caps) | **open** | Second peripheral on the M6 pattern; IRQ-cap path | K1 useful; ADR-0013 pattern |
| K10 | Supervisor lifecycle (restart, creator exit) | **open** | Product supervisor can restart/reap without ad-hoc demos | Builds on 0018/0025 |

## P — product operating system

| ID | Track | Status | Done when (sketch) | Typical deps |
| --- | --- | --- | --- | --- |
| P1 | Multi-agent product image beyond beacon | **done (QEMU)** first slice (beacon + chirp in store) | Product store n≥2; both run via console endpoint | ADR-0027/0029 |
| P2 | Storage path (block + load/persist) | **open** | Persist or load agent/data without rebuild-only workflow | Often after K6 |
| P3 | Network agent + caps | **open** | Network I/O only via granted caps; no ambient net | K1/K9 helpful |
| P4 | Display/input product path | **open** | Product-grade path (may graduate `debug-display` discipline) | Device agents |
| P5 | Naming / discovery / system services | **open** | Endpoints findable without hard-coded oracle wiring | K3 useful |
| P6 | Compose/audit tooling | **done (QEMU)** first slice (pack / inject / inspect) | Host tools for store composition and audit | P1 |

## Standing watches (not completeness tracks)

| Work | Done when | Issue |
| --- | --- | --- |
| **ADR-0020 expiry watch** | XPT2046 lands and `SpiDevice` gets a caller, or the trait goes and ADR-0020 is superseded | [#14](https://github.com/gianlucamazza/harbor-kernel/issues/14) |

## Out of model (permanent non-goals)

These are **not** completeness tracks ([ADR-0026](adr/0026-kernel-and-product-completeness.md)):

- Linux / POSIX / glibc compatibility
- Hiding platform firmware blobs ([`blobs.md`](blobs.md))
- Multi-tenant cloud hypervisor (unless a future ADR owns it)

## How to extend

1. Add a row (or split a slice) in this file first.
2. Write/accept a design ADR before boundary code.
3. Land code + gates; flip status only with evidence named in the row.
4. Point GitHub tracking ([#17](https://github.com/gianlucamazza/harbor-kernel/issues/17)) at this file — do not invent a second status table.
