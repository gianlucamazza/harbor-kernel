---
id: 0016
title: EL0 session protocol — one global slot, prose contract, named successor
status: superseded
date: 2026-08-06
accepted: 2026-08-06
superseded: 2026-08-06
related: [0017, 0018]
---

# ADR-0016: The EL0 session protocol

## Superseded

**Superseded** (2026-08-06) by [ADR-0017](0017-el0-capability-abi.md) and
[ADR-0018](0018-agent-fault-policy.md), accepted the same day. This ADR named
both as its successors before either existed; they now exist, and the lifecycle
in [`README.md`](README.md) says an accepted ADR is changed by a successor, not
by editing it. So the text below is left exactly as it was accepted.

**What it decided that is no longer in force:** decision 1 (one session at a
time, machine-wide) and decision 2 (the nine globals stay). ADR-0017 §1 replaces
both — session state moves into the `Tcb`, reached through a single published
pointer, and two agents may be live at EL0. The "Successor" section below is the
change that happened.

**What survives it,** and is worth reading here rather than rediscovering:
decisions 3, 5 and 6 are still the protocol (`end_session` is `unsafe`, an IRQ
resumes by re-execution with no software `+4`, an unknown SVC ends the session
without inventing a behaviour), and the _reason_ for `static mut` in decision 2
is unchanged — a `SyncCell` has no linker-visible name for `adrp`/`add` to load.
ADR-0017 does not remove `static mut`; it reduces nine of them to one.

## Acceptance status

**Accepted** (2026-08-06), as a record of a decision the code had already made.
Written during the file-by-file audit of `src/arch/aarch64/`, which is where it
became clear that the agent boundary — the most consequential interface in the
tree — existed only as doc-comments on the module that implements it.

This ADR does not change the protocol. It states it, states what it costs, and
names what replaces it. [ADR-0014](0014-ttbr-split-m5.md) covers the _TTBR
regime_ for EL0; nothing covered the _protocol_.

## Context

`arch::el0` is how this kernel enters user mode and how it comes back. Every
agent runs through it. Its shape was arrived at incrementally across M5, M5-P
and M6, and each step was reasonable, but the result was never written down as a
decision:

- Session state is **nine `static mut` globals** — `EL0_SAVED`, `EL0_USER_TTBR`,
  `EL0_CAN_RESUME`, `el0_kernel_ttbr0`, `el0_run_sp`, `EL0_ENTRY_SPSR`,
  `EL0_ESR`, `EL0_FAR`, `EL0_KIND`. One slot for the whole machine.
- `src/sync.rs` argues at length that `static mut` is unacceptable in edition 2024. Nothing said why this module is different.
- `end_session()` was a **safe** `pub fn` that clears `el0_kernel_ttbr0` — the
  value `vectors.s` requires to be non-zero on every lower-EL exception.
  Breaking the vector path's precondition was ordinary safe Rust.
- The rule "no `yield_now` inside a session" existed only as a property of how
  `agent::run_user_prog_resuming` happens to be written.

## Decision

**1. One session at a time, machine-wide.** The globals are a single slot and
the protocol says so. `enter` opens a session, an outcome that is not resumable
or `end_session` closes it, and nothing may open a second one in between.

**2. The globals stay, and the reason is stated.** The assembly reaches them by
symbol name (`adrp`/`add` against `EL0_SAVED`, `el0_kernel_ttbr0`,
`EL0_ENTRY_SPSR`), from this module and from `vectors.s`. A `SyncCell` has no
linker-visible name to load, and wrapping them in `UnsafeCell` would move the
same raw access one layer down while making the offsets `el0_resume` hard-codes
depend on a layout Rust does not promise. This is a real constraint, not a
preference — but it means the protection is prose, and the module says that too.

**3. `end_session` is `unsafe`.** Its obligation is "no session is live". This
is the one place where the missing type-level protection had a cheap partial
substitute, and it was not being used.

**4. No `yield_now` inside a session.** Unenforced. Today it holds because the
whole resume loop runs inside `cpu::without_irqs`, which no yield can survive
crossing. A yield inside a session would let a second agent overwrite the first
one's saved context, and `el0_run_sp` would restore the wrong stack.

**5. An IRQ resumes by re-execution.** `ELR` is left at the interrupted
instruction, because the architecture re-executes it. A software `+4` — correct
for SVC, where `ELR` is already past — would silently drop one user instruction
per interrupt.

**6. Unknown SVC ends the session.** It does not invent a behaviour, and it does
not panic: an agent asking for something this kernel does not implement is the
agent's error, not the kernel's.

## Consequences

### Positive

- The contract is checkable against the code, and `end_session` now enforces its
  own half of it.
- The single slot is honest about what the "concurrent agents" evidence proves:
  two agents each holding a prepared address space, entering EL0 one at a time.

### Negative / debt

- **Two agents cannot be live at EL0.** Not a scheduling limitation — a
  structural one. `Tcb` has no address space and no user frame, so there is
  nowhere else for this state to live.
- **An EL0 session is an uninterruptible region** from the scheduler's point of
  view: IRQs are masked at EL1 for its whole duration.
- **Blocking syscalls are impossible.** A syscall that waits would have to
  yield, and rule 4 forbids it.
- **A driver agent cannot wait on an interrupt**, which is why PL011 RX
  ownership is poll-based (ADR-0013) rather than IRQ-driven.

### Gates that catch reversal

| Reversal                                   | Gate                                              |
| ------------------------------------------ | ------------------------------------------------- |
| A second session opened while one is live  | Nothing. This is the gap; see successor           |
| `end_session` made safe again              | `cargo build` — the callers carry `unsafe` blocks |
| Software `+4` added to the IRQ resume path | `make boot-check`: the PL011 agent's byte count   |
| Unknown SVC given a behaviour              | `boot-check` asserts the refusal is counted       |
| Session state read outside a session       | Nothing. Prose only                               |

Two of five are "nothing", which is the honest measure of this ADR: it documents
a protocol whose central invariant no gate can currently see.

> Both "nothing" rows are closed by the successor: with per-task session state
> there is no second session to open, and the `CURRENT_EL0` assertion of
> ADR-0017 §1 is what makes _session state read outside a session_ visible. This
> is what naming the gap bought.

## Successor

**Move EL0 session state into the TCB.** That is the change this ADR exists to
make legible, and it is the precondition for preemption, for blocking syscalls,
and for a driver agent that waits on an interrupt rather than polling. It is
named in the roadmap as M7 and needs its own ADR, together with the EL0
capability ABI — `SYS_PUTC` currently grants the kernel console to any agent
with no capability check at all, which is a separate hole in the same boundary.

## Alternatives rejected

| Alternative                             | Why not                                                                                                                                                         |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A `Session` token owning `resume`/`end` | The right shape, and it does not fix the single slot — the globals stay global. Doing it now spends the churn without the benefit; it belongs with the TCB move |
| Wrap the globals in `SyncCell`          | The assembly needs linker-visible symbols; the wrapper would be decoration over the same raw access                                                             |
| Keep `end_session` safe                 | It clears the value `vectors.s` requires. A safe function should not be able to make the next fault unrecoverable                                               |
| Panic on unknown SVC                    | Punishes the kernel for the agent's mistake, and an agent is not trusted input                                                                                  |

## Related

- [0017](0017-el0-capability-abi.md) — successor: session state in the TCB and
  the EL0 capability ABI
- [0018](0018-agent-fault-policy.md) — successor: what happens when an agent
  faults
- [0014](0014-ttbr-split-m5.md) — the TTBR regime this protocol runs under
- [0006](0006-cooperative-execution-model.md) — why no preemption, which rule 4
  currently leans on
- [0013](0013-narrow-device-windows.md) — the poll-based device ownership that
  rule 4 forces
