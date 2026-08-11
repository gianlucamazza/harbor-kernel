# Foundation history (M0–M8)

**Historical record.** This page holds the milestone narrative of the H0
foundation: what each M/P milestone had to show, what closed, and against which
evidence. It was split out of [`architecture.md`](architecture.md) so that
document can describe how Harbor works **today** without thirty kilobytes of
closed slices in front of it.

Nothing here is live planning. Current status is
[`roadmap.md`](roadmap.md) (K/P tracks, single source of truth); the normative
model is [`architecture.md`](architecture.md); the evidence index is
[`verification.md`](verification.md).

The foundation is **closed on Pi 4B**: M0–M8 plus the parked-wait policy of
ADR-0024/0025. Rows marked **done (HW)** were observed on real silicon with a
serial transcript, not in emulation.

## Milestones

| ID  | Deliverable                                              | Status                                                              |
| --- | -------------------------------------------------------- | ------------------------------------------------------------------- |
| M0  | Hello UART + echo                                        | **done**                                                            |
| M1  | Exceptions + timer IRQ ticks                             | **done** (HW)                                                       |
| M2  | MMU + kernel heap (+ atomics after attrs)                | **done** (HW)                                                       |
| P0  | Idle (WFI) + UART RX IRQ + ring                          | **done** (HW)                                                       |
| P1  | W^X + guard page + free-list `GlobalAlloc`               | **done** (HW, fault-probed)                                         |
| P2  | Early MMU, softfloat, build-enforced gates               | **done** (HW)                                                       |
| P3  | Layout validation, runtime `map` + TLB maintenance, ADRs | **done** (HW)                                                       |
| P4  | Exception stack, refused frees, fatal map failure        | **done** (HW, fault-probed)                                         |
| M3  | Cooperative tasks                                        | **done** (HW, fault-probed)                                         |
| M4  | IPC + capabilities                                       | **done (HW)**                                                       |
| M5  | EL0 agents                                               | **done (HW)**                                                       |
| M6  | Driver-as-agent                                          | **done (HW)** page map + FR + kill; **RX own done (HW)** 2026-08-06 |
| M7  | Slot ABI + blocking recv + manifest loader               | **done (HW)** 2026-08-07                                            |
| M8  | Console endpoint + beacon + parked-wait cancel           | **done (HW)** 2026-08-08 stamp                                      |

**M** milestones add capability. **P** milestones add protection or evidence and
add no capability at all: they are numbered separately because "the kernel can
now do X" and "the kernel can now be trusted about X" are different claims, and
mixing them lets the second silently stand in for the first. A P milestone is
work that would be invisible in a demo.

These **M/P milestone numbers are the foundation-era vocabulary** and do not
continue: live work is numbered **K** (kernel) and **P** (product) in
[`roadmap.md`](roadmap.md). The two `P` letters are unrelated, which is one more
reason this record lives apart from the roadmap.

"done (HW)" means the deliverable was observed working on a Raspberry Pi 4B, not
merely in QEMU. The distinction earned its place: emulation booted a kernel that
hung on silicon, because TCG's exclusive monitor ignores memory attributes. See
[`verification.md`](verification.md). P4 met it in three parts: the board boots
with the split stacks and takes timer IRQs — which can only arrive through the
EL1t vector entries — and both fault probes were re-run at their new addresses.

### What each planned milestone needed, and how it was judged done

The done column above was earned against a stated observable. The same standard
applies forwards, or it is not the same standard.

| ID  | Needs first                                                                                                                                                                                                                                                | Done when                                                                                                                                                                                                                                                                            |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| M3  | [ADR-0006](adr/0006-cooperative-execution-model.md) (F12 done); per-task heap stack + unmapped guard                                                                                                                                                       | Two tasks yield to each other on hardware and the console shows their output interleaved; each task stack is validated by `mm::layout`; a probe shows one task's overflow faulting rather than reaching another's stack                                                              |
| M4  | [ADR-0008](adr/0008-irq-handler-policy.md) (**accepted**): cookie handlers + wake queue; mailbox ABI                                                                                                                                                       | A message crosses between two tasks that share no memory; a send on a capability the sender does not hold is refused and counted, and the refusal is visible on the console; IRQ wakes use the ADR-0008 queue only                                                                   |
| M5  | [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md) + [ADR-0014](adr/0014-ttbr-split-m5.md) (TTBR0 v1); multi-role prep                                                                                                                             | A task runs at EL0 in its own `TTBR0`; an EL0 write to a kernel address takes a permission fault with the ESR recorded here, the way W^X was; `SVC` returns to EL1 and back                                                                                                          |
| M6  | M5 done; [ADR-0013](adr/0013-narrow-device-windows.md) (**accepted**); F26                                                                                                                                                                                 | EL0 agent maps **only** the PL011 page, touches the device, is destroyed (kill); kernel console/ticks continue. RX ownership (poll + real bytes) was a post-v1 product slice, closed on silicon 2026-08-06 with [issue #1](https://github.com/gianlucamazza/harbor-kernel/issues/1). |
| M7  | [ADR-0017](adr/0017-el0-capability-abi.md) (EL0 capability ABI) and [ADR-0018](adr/0018-agent-fault-policy.md) (agent fault policy), both **accepted** 2026-08-06 — which is what unblocks the milestone under [ADR-0001](adr/0001-multi-role-analysis.md) | Two EL0 agents exchange a message neither can forge; one of them faults; its creator handles the fault and the other keeps running; the kernel stays alive — **on silicon**, with a serial transcript. **Done (HW) 2026-08-07** in one boot across all four slices                   |

M3 is **done (HW)**. [ADR-0006](adr/0006-cooperative-execution-model.md) is
**accepted**. Observed on **Pi 4B silicon**: interleaved `task-a`/`task-b`,
unmap smoke, and a scheduled task-stack overflow that took a **translation
fault** in its own guard with peers live
([verification.md](verification.md#m3-cooperative-tasks-hardware)). QEMU remains
gated by `boot-check`. Desk multi-role pass:
[reviews/2026-08-04-m3-incremental.md](reviews/2026-08-04-m3-incremental.md).
Inventing preemption or `link.ld` task stacks is a reversal of the ADR.

M4 is **done (HW)**. [ADR-0008](adr/0008-irq-handler-policy.md) is **accepted**.
QEMU `boot-check` and Pi 4B boot (2026-08-05) show message cross + refuse
count ([verification.md](verification.md#m4-ipc--capabilities)).

M5 is **done (HW)**. [ADR-0012](adr/0012-frame-allocator-for-address-spaces.md)
and [ADR-0014](adr/0014-ttbr-split-m5.md) are **accepted**. S0–S4: named frame
pool, `AddressSpace` prepare (kernel clone + user window), one-shot
`arch::el0::run` (`switch_ttbr0` sole path), SVC + EL0 store-to-kernel fault
probes, destroy without pool leak. QEMU `boot-check` and Pi 4B PL011 (2026-08-05)
show the same oracles
([verification.md](verification.md#m5-el0--address-spaces)).

## Closed slices

### Closed (HW) — through multi-SVC / M6 v1 map

| Slice                                                   | Status        | Evidence                                                   |
| ------------------------------------------------------- | ------------- | ---------------------------------------------------------- |
| **M5-P1…P3**                                            | **done (HW)** | scheduled EL0, SVC refuse, dual AS                         |
| **M6-D0** [ADR-0013](adr/0013-narrow-device-windows.md) | **accepted**  | 2026-08-05                                                 |
| **M6 v1** PL011 page + FR + kill                        | **done (HW)** | `pl011-agent: FR read + svc ok` / `killed ok`              |
| **Agent shell** + concurrent dual agent                 | **done (HW)** | `agents: concurrent ok`                                    |
| **SVC resume**                                          | **done (HW)** | `enter`/`resume`/`end_session`; `el0-task: resume pings=2` |
| Preferred ELR for SVC                                   | documented    | AArch64 ELR already past SVC — no software `+4`            |

Pi 4B stamp detail: [verification.md §M5-P / M6](verification.md#m5-p--m6-post).

### Closed (HW) — M7, stamped on silicon 2026-08-07 00:05

One boot carrying all four slices. Transcript and what the ordering proves:
[verification.md §M7 closed on silicon](verification.md#hardware-evidence-m7-closed-on-silicon-2026-08-07).

| Slice                                      | Status        | Evidence                                                                                                                                                                                                                                           |
| ------------------------------------------ | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **1 — EL0 session state in the `Tcb`**     | **done (HW)** | nine `static mut` → one published `CURRENT_EL0`; no panic across five agents in four tasks, and deleting the publish from the switch panics on the first spawned-task entry                                                                        |
| **2 — `SYS_SEND` / `SYS_RECV` by slot**    | **done (HW)** | `el0-ipc: sent slot=0 tag=7 a=42` → `*el0-ipc: got payload via EL0 recvs=1`; the receiving agent moves the payload into `SYS_PUTC` itself, so the `*` is the message and not a status code                                                         |
| **2 — authority refused on the good path** | **done (HW)** | `el0-ipc: refused slot=1 authority=2`; a full mailbox counts as `full`, never as authority                                                                                                                                                         |
| **3 — `SYS_PUTC` behind a capability**     | **done (HW)** | `console: capability minted`, then `el0-ipc: console denied, printed nothing` — and the byte that agent tried to print is asserted **absent** from the log                                                                                         |
| **4 — fault policy** (ADR-0018)            | **done (HW)** | `agent faulted esr=0x9200004f far=0x80000 faults=1` then `creator alive after fault`, with the peer completing 22 ms later; `SessionEnd` is `#[must_use]` and has been seen to fail a build                                                        |
| **The done-when, end to end**              | **done (HW)** | two EL0 agents with different capability tables exchange a message neither can forge, one faults, its creator handles it, the other completes, the kernel keeps ticking                                                                            |
| Blocking `SYS_RECV`                        | **done (HW)**   | The agent parks and a peer send wakes it; the oracle spawns the receiver **first** and it still gets the payload, so ordering by construction is gone. `SYS_TRY_RECV` keeps the non-blocking path and is the only producer of `Status::Empty` left ([ADR-0022](adr/0022-blocking-recv-and-the-mask-that-travels.md)) |

Cost: `pool=496` at the concurrent peak and `pool=512` after the kill, identical
to the pre-M7 sessions. Four slices, no frames.

### Closed (HW) — issue #1, stamped on silicon 2026-08-06

Every row below was QEMU-only until the hardware session of 2026-08-06.
Transcript and the four register-level claims:
[verification.md §the four changes of 2026-08-05](verification.md#hardware-evidence-the-four-changes-of-2026-08-05-closed).

| Slice                          | Status        | Evidence                                                                                                                                                |
| ------------------------------ | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **EL0 IRQ save/resume**        | **done (HW)** | architectural re-execute; `el0-task: irq resume irqs=1`                                                                                                 |
| **`SYS_PUTC`**                 | **done (HW)** | imm 2; `el0-task: putc bytes=2`                                                                                                                         |
| **RX poll empty**              | **done (HW)** | `pl011-agent: rx poll empty`                                                                                                                            |
| **RX-owned agent (poll)**      | **done (HW)** | drain off + IMSC mask; `rx own begin/end`, and an injected byte reached the _agent_ while the kernel drain was suspended (`rx poll unexpected putcs=1`) |
| **Real RX bytes**              | **done (HW)** | PL011 **LBE** inject; `rx own bytes=2`, intact underneath ~3500 injected bytes                                                                          |
| **Kill restores kernel drain** | **done (HW)** | `resume_rx` + `killed ok`; idle ticks ran to 270 with no storm                                                                                          |
| Kernel TX / panic              | preserved     | TX never handed to agent                                                                                                                                |

QEMU gate: `make boot-check` / `scripts/boot/qemu-boot-check.sh` (all of the above
oracles). It has three outcomes, not two: `timer: MISSED` is corroborated
against the host CPU the emulator received, and reports **INDETERMINATE**
(exit 3) rather than a red it cannot attribute.

### Closed — ADR-0019 (rule 7 absolute)

| Slice                           | Status        | Evidence                                                                                                                                                                                                                                                                                                                                                       |
| ------------------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`CURRENT_EL0` → `AtomicPtr`** | **done (HW)** | same `adrp`/`add`/`ldr` symbol; `Release` publish / `Acquire` load. Stamped on silicon 2026-08-07 10:24 — the plain `ldr` in `vectors.s` sees the published pointer on every EL0 exception, with zero panics and zero `no published session` ([verification](verification.md#hardware-evidence-main-after-adr-0019--the-atomic-on-the-vector-path-2026-08-07)) |
| **`make no-static-mut`**        | **done**      | greps `src/` for declarations; prerequisite of `make check`                                                                                                                                                                                                                                                                                                    |
| Rule 7 exception                | **gone**      | no `static mut` remains; ADR-0016/0017 keep their false premise text (immutable) and point here                                                                                                                                                                                                                                                                |

### Closed — threat model ([`SECURITY.md`](../SECURITY.md))

| Slice                        | Status   | Evidence                                                                                                           |
| ---------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------ |
| **Threat model + reporting** | **done** | Root [`SECURITY.md`](../SECURITY.md): TCB, attacker, authority surface, claims with gates, residual non-guarantees |
| Bound to M7 authority        | **yes**  | Slot ABI, console denied-by-default, fault policy, refusal counters — as of silicon 2026-08-07                     |

### Closed — M8 console endpoint (HW) 2026-08-07

| Slice | Status | Evidence |
| --- | --- | --- |
| EL1 console server drains the endpoint | **done (HW)** | [`verification.md` §M8](verification.md#hardware-evidence-m8-console-endpoint-closed-on-silicon-2026-08-07); design: [`design/m8-console-endpoint.md`](design/m8-console-endpoint.md) |
| Product manifest carries the beacon | **done (HW)** | same transcript + product QEMU gate |
| `SYS_PUTC` removed; denied-by-default preserved | **done (HW)** | mute refusals=2; refuse count=5; syscall gate |

### Closed — parked-task policy (ADR-0024 / 0025)

| Slice | Status | Evidence |
| --- | --- | --- |
| Visibility (`blocked_count` / `block_events`) | **done (HW)** | [ADR-0024](adr/0024-parked-task-visibility.md); [verification §](verification.md#parked-task-visibility-and-cancel-closed-on-silicon-adr-0024--0025-2026-08-07) |
| Supervisor `cancel_blocked` → `Cancelled` | **done (HW)** | [ADR-0025](adr/0025-cancel-blocked-wait.md); `ipc: reaped cancelled` on silicon |
| Last-SEND-hold auto-reap (ephemeral) | **done (QEMU)** | [ADR-0031](adr/0031-k2-last-send-hold-auto-reap.md); timeout residual still open |
| Timeout / deadline queue | **open (K2 residual)** | Not done by ADR-0025/0031 |

Issue [#13](https://github.com/gianlucamazza/harbor-kernel/issues/13) is **closed**
for visibility + cancel. [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md)
(agent = driver + EL0 program) is accepted so preemption/reaping discussions
name **which half** they mean.

### Closed — F26 EL1 Device residual (risk-accept, 2026-08-07)

[ADR-0013](adr/0013-narrow-device-windows.md) already closed F26 for **agent**
maps (page-sized only). Kernel EL1 may keep coarse Device windows until a
P-pass. That P-pass ([#2](https://github.com/gianlucamazza/harbor-kernel/issues/2),
**closed not planned** 2026-08-07) is risk-accepted rather than implemented:

| Layer | Window | Status |
| --- | --- | --- |
| Agent AS | Named page(s) only (`map_device_page`) | **done (HW)** M6 |
| Kernel EL1 | `DEVICE_REGIONS`: 16 MiB peripherals + 16 KiB GIC (`memmap`) | **risk-accepted** — EL1-only TCB; no agent sees the blanket |

Re-open #2 only if a new agent needs a peripheral still covered only by a
blanket, or if an audit shows EL1 stray stores into Device as a live bug class.

## Findings from the foundation review

From [the multi-role review](reviews/2026-08-04-multi-role.md). Findings not
listed here blocked nothing and are tracked in that report alone.

Status for all thirty lives in the review itself, assigned by the 2026-08-06
audit and verified against the code — `architecture.md` used to track six of
them while the report tracked none. **None is still open.** The last was F23,
board topology encoded in `arch` through the early map: closed on 2026-08-06 by
moving the map to `src/mm/early.rs`, where the seam between board and CPU has a
name instead of a hiding place, and by `make arch-board-free`, which sees the
way of knowing a board that `make layering` cannot — writing its addresses out
by hand.

| Finding | Blocked | Why it is closed |
| --- | --- | --- |
| F12 | — (resolved) | Closed by [ADR-0006](adr/0006-cooperative-execution-model.md); the ADR was the deliverable |
| F18 | — (resolved) | Absolute `CNTP_CVAL` deadlines + missed-tick counter; pure cooperative yield never depended on it |
| F13 | — (resolved) | Shape accepted: `Handler = fn(IrqCookie)` + IRQ→voluntary wake queue — [ADR-0008](adr/0008-irq-handler-policy.md); code lands with first M4 PR |
| F26 | — (resolved M6 v1) | [ADR-0013](adr/0013-narrow-device-windows.md) **accepted**; agent maps are page-sized named windows only; kernel coarse Device may remain until a P-pass |
| F15 | — (resolved) | Risk-accepted: board truth is BSP constants; DTB mapped RO for a future parser — [ADR-0011](adr/0011-dtb-mapped-board-constants-risk-accept.md) |
| F24 | — (resolved) | Layering rules 1–4 are enforced by `make layering`; non-import coupling remains review-only (gate blind spots in verification) |
| F23 | — (resolved) | Early map in `mm::early`; board says which gigabyte is what via `memmap::EARLY_BLOCKS`; `make arch-board-free` refuses a physical range base under `src/arch/` |

## Side-track (not an M/P milestone)

Optional lab **SPI TFT status surface** (Waveshare-class 3.5″ / ILI9486) was
specified in [ADR-0009](adr/0009-optional-spi-tft-debug-console.md),
[ADR-0010](adr/0010-spi-transaction-and-dbi-panel.md), and
[`hardware.md`](hardware.md). It is observability, not agent capability: UART
stays primary; the panel is a structured status sink behind a default-off
feature (`debug-display`). **SPI0, regwidth-16 ILI bring-up, and the status
surface are silicon-closed**
([verification](verification.md#rng200-and-spi0-hardware)). Missing
peripherals soft-fail via `arch::probe` (QEMU RNG hole) rather than a feature
gate. It did not block or redefine M4–M6.

**Retired 2026-08-11** ([ADR-0094](adr/0094-retire-debug-display.md)): the
drivers compiled in every `make check` and were executed by nothing, and no
product composition named a panel. K9 landed its driver-as-agent track on the
RNG200 instead. The pure half (`kernel_core::{display, textgrid, font8x8,
spi}`) survives; the HAT binding does not.
