# Architecture Decision Records

For the broader documentation map, ownership rules and status vocabulary see
[`../README.md`](../README.md). This index is only the lifecycle and catalogue
for structural decisions.

Structural decisions. Lifecycle:

1. An ADR starts `proposed`, with Context / Decision / Consequences /
   Alternatives.
2. A human accepts it, possibly with refinements → `accepted`.
3. An `accepted` ADR is **immutable**. To change it, write a successor and mark
   the old one `superseded` (linked through `related`).

Numbering is monotonic; never renumber. Prefer a separate `docs:` commit from
the code that follows.

| ID                                                      | Title                                                                             | Status     |
| ------------------------------------------------------- | --------------------------------------------------------------------------------- | ---------- |
| [0001](0001-multi-role-analysis.md)                     | Multi-role analysis as project gate before M3                                     | accepted   |
| [0002](0002-softfloat-kernel.md)                        | Kernel compiled softfloat, FP left trapping                                       | accepted   |
| [0003](0003-early-mmu.md)                               | MMU enabled before any Rust runs                                                  | accepted   |
| [0004](0004-gic-group0-firmware-pin.md)                 | GIC Group 0 with IAR/EOIR, and the firmware pin                                   | accepted   |
| [0005](0005-static-page-table-arena.md)                 | Static page-table arena instead of a frame allocator                              | accepted   |
| [0006](0006-cooperative-execution-model.md)             | Cooperative execution model (M3 tasks)                                            | accepted   |
| [0007](0007-project-identity-harbor-kernel.md)          | Project identity — Harbor and `harbor-kernel`                                     | accepted   |
| [0008](0008-irq-handler-policy.md)                      | IRQ handler policy for cooperative wakes (F13/M4)                                 | accepted   |
| [0009](0009-optional-spi-tft-debug-console.md)          | Optional SPI TFT status surface (ILI9486 HAT)                                     | accepted   |
| [0010](0010-spi-transaction-and-dbi-panel.md)           | SPI transactions and DBI panel streaming                                          | accepted   |
| [0011](0011-dtb-mapped-board-constants-risk-accept.md)  | DTB mapped; board truth compiled-in (F15)                                         | accepted   |
| [0012](0012-frame-allocator-for-address-spaces.md)      | Frame allocator for user address spaces (M5)                                      | accepted   |
| [0013](0013-narrow-device-windows.md)                   | Narrow device MMIO windows (F26 / M6)                                             | accepted   |
| [0014](0014-ttbr-split-m5.md)                           | TTBR regime for M5 (TTBR0 + shared kernel maps v1)                                | accepted   |
| [0015](0015-multi-arch-scaffold.md)                     | Multi-arch scaffold (cfg facade, board features)                                  | accepted   |
| [0016](0016-el0-session-protocol.md)                    | EL0 session protocol (one slot, prose contract)                                   | superseded |
| [0017](0017-el0-capability-abi.md)                      | EL0 capability ABI (slot-indexed authority, session in TCB)                       | accepted   |
| [0018](0018-agent-fault-policy.md)                      | Agent fault policy (kernel ends session, creator decides task)                    | accepted   |
| [0019](0019-no-static-mut.md)                           | No `static mut` — the last one becomes an atomic (rule 7)                         | accepted   |
| [0020](0020-spidevice-contract-without-a-caller.md)     | `SpiDevice`: an adopted contract with no caller; ADR-0010's description retracted | accepted   |
| [0021](0021-agents-as-data-and-the-manifest.md)         | Agents as data + manifest: authority enumerable in one artefact                   | accepted   |
| [0022](0022-blocking-recv-and-the-mask-that-travels.md) | Blocking `SYS_RECV`: an agent parks, and `without_irqs` stops spanning a switch   | accepted   |
| [0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md) | An agent is an EL1 driver task **and** an EL0 program; the driver is what the scheduler runs | accepted   |
| [0024](0024-parked-task-visibility.md)                      | Parked tasks are counted; reclaim/timeout deferred (issue #13 phase 1)                        | accepted   |
| [0025](0025-cancel-blocked-wait.md)                         | Cancel a blocked wait — supervisor reaping without a timeout queue (issue #13 phase 2)        | accepted   |
| [0026](0026-kernel-and-product-completeness.md)             | Completeness of the Harbor kernel and product OS is the project goal                          | accepted   |
| [0027](0027-h1-external-agent-store.md)                     | H1 first slice — external agent store at a fixed physical address                               | accepted   |
| [0028](0028-wait-on-irq.md)                                 | K1 first slice — wait on IRQ cookie (EL1)                                                         | accepted   |
| [0029](0029-agent-store-in-image.md)                        | Agent store lives in the kernel image section (placement)                                         | accepted   |
| [0030](0030-el0-irq-capability.md)                          | K1 remainder — EL0 IRQ notification capability and `SYS_WAIT_IRQ`                                   | accepted   |
| [0031](0031-k2-last-send-hold-auto-reap.md)                 | K2 first slice — last SEND-hold auto-cancel on ephemeral channels                                   | accepted   |
| [0032](0032-k3-channel-revoke.md)                           | K3 first slice — channel revoke and generation recycle                                              | accepted   |
| [0033](0033-k10-supervisor-reap.md)                         | K10 first slice — supervisor reaps a blocked task (restart = re-spawn)                              | accepted   |
| [0034](0034-k9-rng-driver-agent.md)                          | K9 first slice — RNG200 second driver-as-agent (page map)                                           | accepted   |
| [0035](0035-p5-name-registry.md)                             | P5 first slice — EL1 name registry (name to CapId)                                                  | accepted   |

Porting / facade contract (not ADRs): [`../arch-contract.md`](../arch-contract.md),
[`../porting.md`](../porting.md).

Operational reviews (findings, not decisions): [`../reviews/`](../reviews/).

## Why 0002–0006 exist

0001 institutionalised _how_ to review before anything recorded _what_ had been
decided: someone arriving and asking "why softfloat?" had no answer. 0002–0005
cover the four choices that already constrain the running kernel and are
**accepted** with the evidence each ADR names (gates seen red, or silicon, or
both). 0006 records the execution model (finding F12): cooperative tasks, heap stacks
with unmapped guards, no preemption, no IRQ-side switch. The **model** is
accepted and M3 is **done (HW)** (interleaved yield + overflow probe on silicon
— see [`../verification.md`](../verification.md)). Inventing preemption is not
allowed without a successor ADR.

Each names **the gate that would catch its own reversal**, and for several of
them that gate has been seen red — see the mutation table in
[`../verification.md`](../verification.md). 0005 declares a weak console-only
remainder signal; 0006 is now also covered by QEMU interleave/split gates and
hardware ESR evidence.
