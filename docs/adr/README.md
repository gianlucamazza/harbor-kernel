# Architecture Decision Records

For the broader documentation map, ownership rules and status vocabulary see
[`../README.md`](../README.md). This index is only the lifecycle and catalogue
for structural decisions.

Structural decisions. Lifecycle:

1. An ADR starts `proposed`, with Context / Decision / Consequences /
   Alternatives.
2. A human accepts it, possibly with refinements → `accepted`.
3. An `accepted` ADR is **immutable**. To change it, write a successor and mark
   the old one `superseded` (linked through `related`). One narrow exception
   ([ADR-0058](0058-adr-amendments-and-mutation-freshness.md)): a
   reconciliation amendment that aligns stated mechanism with a landed slice,
   marked by an `amended:` frontmatter bump and a note naming the reconciler.

Numbering is monotonic; never renumber. Prefer a separate `docs:` commit from
the code that follows.

| ID                                                           | Title                                                                                        | Status     |
| ------------------------------------------------------------ | -------------------------------------------------------------------------------------------- | ---------- |
| [0001](0001-multi-role-analysis.md)                          | Multi-role analysis as project gate before M3                                                | accepted   |
| [0002](0002-softfloat-kernel.md)                             | Kernel compiled softfloat, FP left trapping                                                  | accepted   |
| [0003](0003-early-mmu.md)                                    | MMU enabled before any Rust runs                                                             | accepted   |
| [0004](0004-gic-group0-firmware-pin.md)                      | GIC Group 0 with IAR/EOIR, and the firmware pin                                              | accepted   |
| [0005](0005-static-page-table-arena.md)                      | Static page-table arena instead of a frame allocator                                         | accepted   |
| [0006](0006-cooperative-execution-model.md)                  | Cooperative execution model (M3 tasks) — IRQ-epilogue rule superseded by 0064/0068 (amended) | accepted   |
| [0007](0007-project-identity-harbor-kernel.md)               | Project identity — Harbor and `harbor-kernel`                                                | accepted   |
| [0008](0008-irq-handler-policy.md)                           | IRQ handler policy for cooperative wakes (F13/M4)                                            | accepted   |
| [0009](0009-optional-spi-tft-debug-console.md)               | Optional SPI TFT status surface (ILI9486 HAT)                                                | accepted   |
| [0010](0010-spi-transaction-and-dbi-panel.md)                | SPI transactions and DBI panel streaming                                                     | accepted   |
| [0011](0011-dtb-mapped-board-constants-risk-accept.md)       | DTB mapped; board truth compiled-in (F15)                                                    | accepted   |
| [0012](0012-frame-allocator-for-address-spaces.md)           | Frame allocator for user address spaces (M5)                                                 | accepted   |
| [0013](0013-narrow-device-windows.md)                        | Narrow device MMIO windows (F26 / M6)                                                        | accepted   |
| [0014](0014-ttbr-split-m5.md)                                | TTBR regime for M5 (TTBR0 + shared kernel maps v1)                                           | accepted   |
| [0015](0015-multi-arch-scaffold.md)                          | Multi-arch scaffold (cfg facade, board features)                                             | accepted   |
| [0016](0016-el0-session-protocol.md)                         | EL0 session protocol (one slot, prose contract)                                              | superseded |
| [0017](0017-el0-capability-abi.md)                           | EL0 capability ABI (slot-indexed authority, session in TCB)                                  | accepted   |
| [0018](0018-agent-fault-policy.md)                           | Agent fault policy (kernel ends session, creator decides task)                               | accepted   |
| [0019](0019-no-static-mut.md)                                | No `static mut` — the last one becomes an atomic (rule 7)                                    | accepted   |
| [0020](0020-spidevice-contract-without-a-caller.md)          | `SpiDevice`: an adopted contract with no caller; ADR-0010's description retracted            | accepted   |
| [0021](0021-agents-as-data-and-the-manifest.md)              | Agents as data + manifest: authority enumerable in one artefact                              | accepted   |
| [0022](0022-blocking-recv-and-the-mask-that-travels.md)      | Blocking `SYS_RECV`: an agent parks, and `without_irqs` stops spanning a switch              | accepted   |
| [0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md) | An agent is an EL1 driver task **and** an EL0 program; the driver is what the scheduler runs | accepted   |
| [0024](0024-parked-task-visibility.md)                       | Parked tasks are counted; reclaim/timeout deferred (issue #13 phase 1)                       | accepted   |
| [0025](0025-cancel-blocked-wait.md)                          | Cancel a blocked wait — supervisor reaping without a timeout queue (issue #13 phase 2)       | accepted   |
| [0026](0026-kernel-and-product-completeness.md)              | Completeness of the Harbor kernel and product OS is the project goal                         | accepted   |
| [0027](0027-h1-external-agent-store.md)                      | H1 first slice — external agent store at a fixed physical address                            | accepted   |
| [0028](0028-wait-on-irq.md)                                  | K1 first slice — wait on IRQ cookie (EL1)                                                    | accepted   |
| [0029](0029-agent-store-in-image.md)                         | Agent store lives in the kernel image section (placement)                                    | accepted   |
| [0030](0030-el0-irq-capability.md)                           | K1 remainder — EL0 IRQ notification capability and `SYS_WAIT_IRQ`                            | accepted   |
| [0031](0031-k2-last-send-hold-auto-reap.md)                  | K2 first slice — last SEND-hold auto-cancel on ephemeral channels                            | accepted   |
| [0032](0032-k3-channel-revoke.md)                            | K3 first slice — channel revoke and generation recycle                                       | accepted   |
| [0033](0033-k10-supervisor-reap.md)                          | K10 first slice — supervisor reaps a blocked task (restart = re-spawn)                       | accepted   |
| [0034](0034-k9-rng-driver-agent.md)                          | K9 first slice — RNG200 second driver-as-agent (page map)                                    | accepted   |
| [0035](0035-p5-name-registry.md)                             | P5 first slice — EL1 name registry (name to CapId)                                           | accepted   |
| [0036](0036-p2-keyed-blob-store.md)                          | P2 first slice — EL1 keyed blob store (on-target put/get)                                    | accepted   |
| [0037](0037-k3-cap-transfer.md)                              | K3 residual — EL1 capability transfer between tasks                                          | accepted   |
| [0038](0038-k10-creator-exit-cascade.md)                     | K10 residual — cascade cancel of blocked children on creator exit                            | accepted   |
| [0039](0039-p5-el0-resolve.md)                               | P5 residual — EL0 SYS_RESOLVE into an empty slot                                             | accepted   |
| [0040](0040-k2-park-timeout.md)                              | K2 residual — park timeout on tick deadlines                                                 | accepted   |
| [0041](0041-el0-cap-transfer.md)                             | K3 residual — EL0 SYS_TRANSFER (self or creator)                                             | accepted   |
| [0042](0042-el0-recv-timeout.md)                             | K2 residual — EL0 SYS_RECV_TIMEOUT                                                           | accepted   |
| [0043](0043-k9-irq-device-agent.md)                          | K9 residual — IRQ-cap device agent (wait path)                                               | accepted   |
| [0044](0044-k5-agent-density.md)                             | K5 first slice — thin task stacks for agent density                                          | accepted   |
| [0045](0045-p2-durable-store.md)                             | P2 residual — durable keyed store region                                                     | accepted   |
| [0046](0046-k4-cooperative-cpu-budget.md)                    | K4 first slice — cooperative CPU budget                                                      | accepted   |
| [0047](0047-k7-asid-isolation-design.md)                     | K7 design — ASID isolation                                                                   | accepted   |
| [0048](0048-k8-smp-design.md)                                | K8 design — SMP (first code: 0070)                                                           | accepted   |
| [0049](0049-deferred-residuals.md)                           | Deferred residuals — peer transfer, resolve-grant, P3/P4, #14                                | accepted   |
| [0050](0050-k7-asid-first-slice.md)                          | K7 first slice — ASID pool, CONTEXTIDR, nG user leaves                                       | accepted   |
| [0051](0051-k4-irq-preemption-design.md)                     | K4 design — IRQ-side preemption (code: 0064/0068)                                            | accepted   |
| [0052](0052-p5-resolve-grant.md)                             | P5 residual — resolve grant (non-ambient SYS_RESOLVE)                                        | accepted   |
| [0053](0053-k3-peer-transfer-design.md)                      | K3 design — peer transfer via task-cap                                                       | accepted   |
| [0054](0054-k3-peer-transfer-first-slice.md)                 | K3 first slice — peer transfer via task-cap                                                  | accepted   |
| [0055](0055-transferable-cap-bands.md)                       | K3 — transferable capability bands                                                           | accepted   |
| [0056](0056-ipc-abi-capacities.md)                           | IPC ABI capacities — canonical numbers                                                       | accepted   |
| [0057](0057-taskcap-lifecycle.md)                            | K3 — task-cap lifecycle invariants                                                           | accepted   |
| [0058](0058-adr-amendments-and-mutation-freshness.md)        | Process — ADR amendments and mutation freshness                                              | accepted   |
| [0059](0059-typed-cap-classification.md)                     | Typed capability classification (CapClass)                                                   | accepted   |
| [0060](0060-syscall-reply-layer.md)                          | Syscall reply layer as a pure machine                                                        | accepted   |
| [0061](0061-refusal-detail-taxonomy.md)                      | Refusal detail taxonomy in x1                                                                | accepted   |
| [0062](0062-taskid-epoch.md)                                 | Epoch in the task identity                                                                   | accepted   |
| [0063](0063-capslots-extraction.md)                          | Capability slots as a pure table                                                             | accepted   |
| [0064](0064-k4-el0-preemption-first-slice.md)                | K4 first code slice — lower-EL (EL0) IRQ preemption                                          | accepted   |
| [0065](0065-platform-self-check.md)                          | Platform self-check — CPU identity decoded, printed, asserted at boot                        | accepted   |
| [0066](0066-sd-media-durable-store.md)                       | P2 — SD media persistence for the durable store (EMMC2 PIO)                                  | accepted   |
| [0067](0067-host-lab-second-isa-intent.md)                   | Host/lab second ISA — QEMU x86_64 intent and non-goals                                       | accepted   |
| [0068](0068-k4-el1-preemption-second-slice.md)               | K4 second code slice — same-EL (EL1) IRQ preemption                                          | accepted   |
| [0069](0069-harbor-host-class-north-star.md)                 | Harbor host-class north star — native primary OS intent                                      | accepted   |
| [0070](0070-k8-smp-first-slice.md)                           | K8 first slice — unpark core 1, idle only                                                    | accepted   |
| [0071](0071-h3-l0-x86-qemu-first-slice.md)                   | H3 L0 — x86_64 QEMU first slice (boot, console, cpu identity)                                | accepted   |
| [0072](0072-hardware-self-discovery-design.md)               | Hardware self-discovery as boot evidence — verify, don't select (first code: 0073)           | accepted   |
| [0073](0073-discovery-first-slice-fdt-report.md)             | Discovery first slice — FDT reader and the `discover:` report                                | accepted   |
| [0074](0074-k8-ipi-wake-second-slice.md)                     | K8 second slice — SGI IPI wake core 1 (no runqueue yet)                                      | accepted   |
| [0075](0075-k8-per-core-queues-design.md)                    | K8 design — per-core ready queues and current (code: 0076)                                   | accepted   |
| [0076](0076-k8-per-core-queues-first-slice.md)               | K8 third slice — per-core queues, dual current, pinned CPU1 worker                           | accepted   |
| [0077](0077-smp-shared-state-discipline.md)                  | SMP shared-state discipline — IrqSpinLock, per-CPU mirrors, honest dual-current              | accepted   |
| [0078](0078-k8-per-core-timer-preemption-design.md)          | K8 design — per-core timer and IRQ preemption on CPU 1 (code: 0079)                          | accepted   |
| [0079](0079-k8-per-core-timer-preemption-first-slice.md)     | K8 fourth slice — per-core timer + EL1 preemption on CPU 1                                   | accepted   |
| [0080](0080-k8-el0-on-cpu1-design.md)                        | K8 design — EL0 sessions and agents with home on CPU 1 (code: 0081)                          | accepted   |
| [0081](0081-k8-el0-on-cpu1-first-slice.md)                   | K8 fifth slice — EL0 sessions and preemption on CPU 1                                        | accepted   |
| [0082](0082-k8-work-stealing-design.md)                      | K8 design — work stealing between per-core ready queues (code: 0083)                         | accepted   |
| [0083](0083-k8-work-stealing-first-slice.md)                 | K8 sixth slice — work stealing first code                                                    | accepted   |
| [0084](0084-k7-residual-policy.md)                           | K7 residual policy — switch-cost evidence, TTBR1 triggers, ASID honesty                      | accepted   |
| [0085](0085-k5-density-residual-design.md)                   | K5 density residual policy — K5-S/H/B split; Mini first code                                 | accepted   |
| [0086](0086-k5-mini-stack-first-slice.md)                    | K5-S first slice — Mini stacks (one page, no unmapped guard)                                 | accepted   |
| [0087](0087-oracle-waits-and-the-hosts-verdict.md)           | Oracle waits in guest time; a starved host gets no verdict                                   | accepted   |
| [0088](0088-product-home-cpu.md)                             | Product multi-core — manifest `home_cpu` and loader pin                                      | accepted   |
| [0089](0089-k5-b-pair-collapse-design.md)                    | K5-B design — pair collapse (session schedulable); no code                                   | accepted   |
| [0090](0090-k10-force-exit-running.md)                       | K10 force-exit Running at a safe point                                                       | accepted   |
| [0091](0091-data-in-lock.md)                                 | Data in the lock — `Mutex<T>` replaces the cell/lock pair                                    | accepted   |

Porting / facade contract (not ADRs): [`../arch-contract.md`](../arch-contract.md),
[`../porting.md`](../porting.md). Lab second-ISA matrix:
[`../design/host-lab-platform-matrix.md`](../design/host-lab-platform-matrix.md).
Native multi-arch + Linux-independence practices:
[`../design/native-multiarch-practices.md`](../design/native-multiarch-practices.md).
Progressive second-ISA (no-debt bar):
[`../design/progressive-isa-practices.md`](../design/progressive-isa-practices.md).
Project scale axes (where code grows):
[`../design/project-topology.md`](../design/project-topology.md).

Operational reviews (findings, not decisions): [`../reviews/`](../reviews/).

## Why 0002–0006 exist

0001 institutionalised _how_ to review before anything recorded _what_ had been
decided: someone arriving and asking "why softfloat?" had no answer. 0002–0005
cover the four choices that already constrain the running kernel and are
**accepted** with the evidence each ADR names (gates seen red, or silicon, or
both). 0006 records the execution model (finding F12): cooperative tasks, heap stacks
with unmapped guards, no preemption, no IRQ-side switch. The **model** is
accepted and M3 is **done (HW)** (interleaved yield + overflow probe on silicon
— see [`../verification.md`](../verification.md)). Inventing preemption was not
allowed without a successor ADR — [0064](0064-k4-el0-preemption-first-slice.md)
(EL0) and [0068](0068-k4-el1-preemption-second-slice.md) (EL1) are that
successor pair: quantum preemption on the IRQ epilogue; device IRQ handlers
still never switch.

Each names **the gate that would catch its own reversal**, and for several of
them that gate has been seen red — see the mutation table in
[`../verification.md`](../verification.md). 0005 declares a weak console-only
remainder signal; 0006 is now also covered by QEMU interleave/split gates and
hardware ESR evidence.
