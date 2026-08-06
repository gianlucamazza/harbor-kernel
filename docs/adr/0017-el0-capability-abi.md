---
id: 0017
title: EL0 capability ABI — slot-indexed authority and session state in the TCB
status: accepted
date: 2026-08-06
accepted: 2026-08-06
---

# ADR-0017: The EL0 capability ABI

## Acceptance status

**Accepted** (2026-08-06), **with one refinement to decision 1** — see the note
inside it. The lifecycle in [`README.md`](README.md) allows an acceptance to
carry refinements, and this one was not cosmetic: as written, decision 1 could
not be implemented without granting `check-layering.sh` its first exception.

Required before M7 by [ADR-0001](0001-multi-role-analysis.md): M7 moves the
authority boundary, and it is the boundary this project exists to defend. This
acceptance is what unblocks M7. [ADR-0016](0016-el0-session-protocol.md) names
this ADR as one of its two successors and is `superseded` as of the same date.

## Context

M7's done-when is _two EL0 agents exchange a message they cannot forge_. Three
things stand between the code and that sentence, and each has been documented as
debt rather than fixed.

**Session state is a single machine-wide slot.** `arch::el0` keeps nine
`static mut` globals — `EL0_SAVED`, `EL0_SAVED_SP_EL0`, `EL0_USER_TTBR`,
`EL0_CAN_RESUME`, `el0_kernel_ttbr0`, `el0_run_sp`, `EL0_ENTRY_SPSR`, `EL0_ESR`,
`EL0_FAR`, `EL0_KIND`. ADR-0016 recorded why they are `static mut` (the
assembly reaches them by linker-visible name) and that one slot means one live
session, machine-wide. Two agents cannot be at EL0 at once, which makes M7's
sentence unsayable, and no blocking syscall can exist because a syscall that
waits would have to yield out of a session that is not re-entrant.

**The syscall ABI has no argument space for authority.** Three immediates —
`SYS_PING`, `SYS_EXIT`, `SYS_PUTC` — and none of them names an object.

**`SYS_PUTC` is an unchecked grant.** Any agent that executes `svc #2` writes to
the kernel console. There is no capability, no check, and no counter: it is the
one thing in this kernel an agent obtains by asking.

Meanwhile M4 already built the mechanism this needs. `Tcb` carries
`caps: [Option<CapId>; MAX_CAPS_PER_TASK]` — a per-task table of unforgeable
capabilities — and `sched::current_holds` already asks _was this given to you_
as a separate question from _does it still name a live endpoint_
(`kernel_core::ipc::Table::send`). That machinery is used by EL1 callers only.
EL0 has never been able to reach it.

## Decision

### 1. EL0 session state moves into the TCB, reached through one pointer

The nine globals become fields of an `El0Session`, held as `Option<El0Session>`
in the `Tcb`. The assembly's requirement is a _linker-visible symbol_, not a
static object: it is satisfied by a single

```rust
static mut CURRENT_EL0: *mut El0Session
```

which the scheduler publishes on every switch, and which `vectors.s` and
`el0_resume` dereference before applying the field offsets they already
hard-code. Nine `static mut` become one.

This is what makes everything else in this ADR possible, and it is the
structural precondition ADR-0016 named. It also repays part of that ADR's debt
directly: the invariant _no second session while one is live_ stops being prose,
because a second session is a second TCB and the states are per-task.

> **Refinement at acceptance.** This decision was proposed as
> `static mut CURRENT_TCB: *mut Tcb`, and that form cannot be built. `Tcb` lives
> in `src/sched`, the symbol has to live in `arch` — it is what `el0_resume`
> loads with `adrp`/`add` — and rule 3 of [`architecture.md`](../architecture.md)
> forbids `arch` from seeing `sched`. `check-layering.sh` enforces that on every
> import edge and has no exceptions; buying one here would cost more than the
> indirection saves, a day after that same gate's sibling caught F23.
>
> The way out is that the nine globals are **all `arch` concepts** — saved GPRs,
> TTBR, SPSR, ESR/FAR. So `El0Session` is owned by `arch::el0` along with the
> pointer, and `Tcb` holds an `Option<El0Session>`, which is allowed because
> `sched` may see `arch`. The substance of the decision is untouched: nine
> symbols become one, published on switch, offsets asserted at compile time.
> Only the pointee changes, from the task to the task's session.

`static mut` remains, and remains for the same reason ADR-0016 gave — `SyncCell`
has no name a `adrp`/`add` pair can load. What changes is the count and the
blast radius. The field offsets the assembly hard-codes gain a compile-time
`offset_of` assertion, the way `Context` already has one (see the M3 entry in
`verification.md`: swapping `x30` and `sp` failed two `offset_of` asserts at
compile time, while the size assert alone stayed green).

### 2. An agent names a capability by slot index, not by `CapId`

`SYS_SEND` and `SYS_RECV` take a small integer in `x0`: an index into the
calling task's own `Tcb.caps`. The kernel translates:

```rust
let slot = user_x0 as usize;
let cap = tcb.caps.get(slot).copied().flatten()
    .ok_or(SendError::BadCap)?;
```

Out of range, or an empty slot, is a refusal counted by the _authority_ counter
— which exists separately from `full` and `state` precisely so this class can be
distinguished from flow control.

The point is what an agent **cannot express**. With a raw `CapId` an agent can
name any capability in the machine and is stopped by a check; with a slot index
there is nothing outside its own array to name. Unforgeability becomes
structural rather than enforced, which is the difference between an
object-capability system and an access-control list. It is also the property
that survives a bug in the check.

Note the asymmetry with EL1, which keeps passing `CapId` and keeps being
verified by `current_holds`. That is deliberate: EL1 is inside the TCB, EL0 is
not, and the boundary is exactly where the stronger form is worth its cost.

### 3. `SYS_PUTC` takes a slot too, and is denied by default

The console becomes an ordinary capability held in a slot. An agent that was not
granted one gets a refusal counted as an authority violation.

**Denied by default, and one bootstrap agent is deliberately denied it.** A
capability that nobody is ever seen to lack is a protection nobody has seen
fire, which this project's own doctrine — every gate must be seen red — rejects.
So the boot log must contain a refusal on the good path, and `boot-check` must
assert it.

This is the first case in this kernel where an agent loses something it
currently has. That is the intended direction.

### 4. `SYS_PUTC` is transitional, and its successor is named

Because of decision 2, `SYS_PUTC(slot)` is already isomorphic to
`SYS_SEND(slot)` on a console endpoint. The only difference is who drains the
message: today the kernel, tomorrow an EL1 console server.

**M8 replaces it.** The same slot becomes a send capability on a console
endpoint, `SYS_PUTC` is removed, and the agent-side ABI does not change — only
what sits on the other side. Doing it now would mean writing the server first,
against a mailbox four messages deep, and losing direct TX during bring-up,
which is the one thing that makes an agent observable before the scheduler is
trusted.

The ABI after this ADR:

| imm | name       | `x0` | Authority                                 |
| --- | ---------- | ---- | ----------------------------------------- |
| 0   | `SYS_PING` | —    | none required                             |
| 1   | `SYS_EXIT` | —    | none required                             |
| 2   | `SYS_PUTC` | slot | console capability — **transitional, M8** |
| 3   | `SYS_SEND` | slot | `CapRights::SEND` on the named endpoint   |
| 4   | `SYS_RECV` | slot | `CapRights::RECV` on the named endpoint   |

Mailbox depth (4), mailbox count (8) and endpoint count (16) become part of this
ABI rather than remaining implementation constants: an agent that assumes a
deeper mailbox breaks when it fills, and `src/ipc/mod.rs` already says outright
that no ADR states them. This one does.

## Consequences

### Positive

- Two agents can be live at EL0, which is what M7's done-when requires.
- Blocking syscalls become possible: with per-task session state, a syscall may
  park the task and resume it later. `SYS_RECV` is the first that needs it.
- The authority an agent has is exactly the contents of a four-entry array,
  enumerable by reading one struct. That is the property a threat model needs
  and cannot currently state.
- Nine `static mut` become one, with compile-time assertions on the offsets the
  assembly depends on.

### Negative / debt

- **The ABI is not stable and this ADR does not pretend otherwise.** `SYS_PUTC`
  is explicitly transitional.
- **`MAX_CAPS_PER_TASK` is 4.** A small number chosen for M4, now load-bearing
  at the boundary. An agent needing a fifth capability has no path.
- **Slot indices are not capabilities.** Slot 0 of one task and slot 0 of
  another name different things, so a slot index is meaningless if passed
  between agents. Capability _transfer_ is out of scope here: grants happen at
  creation, by the creator, and nothing delegates at runtime.
- **The `CURRENT_EL0` pointer is a new single point of failure.** A stale value
  after a switch means the wrong session state, and the compile-time offset
  assertions do not cover _when_ it is published, only _where_ the fields are.

### Gates that catch reversal

| Reversal                                       | Gate                                                                                                               |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `SYS_PUTC` granted unconditionally again       | `make boot-check`: the deliberately-denied agent's refusal line disappears and the authority count drops           |
| Slot index replaced by a raw `CapId`           | `cargo build` — the syscall handler signature changes; and the host test asserting an out-of-range slot is refused |
| A slot index accepted beyond the array         | Host test in `kernel-core`: `get(slot)` bound, both sides of it                                                    |
| Session state returned to machine-wide globals | `make boot-check`: the two-agent exchange stops interleaving                                                       |
| An assembly field offset drifts                | `offset_of` assertion, at compile time                                                                             |
| `CURRENT_EL0` not republished on a switch      | Runtime assert on the EL0 entry path (M7 slice 1). See below                                                       |

That row said "nothing yet" when this ADR was proposed, and it was the one that
mattered most: a stale pointer is silent until an agent reads another agent's
saved registers. Acceptance closes it by requiring the fix **in the same commit
as the risk** — the switch path publishes, and the EL0 entry path asserts that
`CURRENT_EL0` is the current task's session before it `eret`s. It is a cheap
check on a path already inside `without_irqs`, and shipping the pointer without
it would be shipping a single point of failure with a note about it.

## Alternatives rejected

| Alternative                                      | Why not                                                                                                                                                                                        |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Raw `CapId` from EL0, checked against the table  | Uniform with EL1 and strictly weaker: an agent can _name_ another's capability and is stopped by a check rather than by structure. A check can have a bug; an array bound is the bug's absence |
| A per-address-space cap table (seL4 badges)      | The right answer if an address space ever holds several tasks. Today it holds one, and it would mean two authority tables coexisting — `Tcb.caps` for EL1 and another for EL0                  |
| Nine `static mut` → nine `[T; MAX_TASKS]` arrays | No pointer indirection, but nine linker symbols stay and every access costs an index multiply in assembly. The pointer costs one load and removes eight symbols                                |
| Keep `SYS_PUTC` unchecked, fix it in M8          | It is the only unchecked grant in the kernel, and M7 is the milestone whose subject is authority. Deferring it means M7 ships with its own counter-example                                     |
| Remove `SYS_PUTC` from EL0 now                   | Architecturally the cleanest and premature: no console server exists, the mailbox is four deep, and direct TX is what makes an agent observable during bring-up                                |
| Serialize EL0 (one agent at a time, enforced)    | Keeps the current shape and makes M7's done-when unreachable by construction. It is the debt ADR-0016 recorded, not a way to pay it                                                            |

## Related

- [0016](0016-el0-session-protocol.md) — the protocol this replaces; names this
  ADR as its successor
- [0018](0018-agent-fault-policy.md) — the other successor: what happens when an
  agent faults
- [0014](0014-ttbr-split-m5.md) — the TTBR regime an EL0 session runs under
- [0006](0006-cooperative-execution-model.md) — why a blocking syscall needs
  per-task state before it can exist
