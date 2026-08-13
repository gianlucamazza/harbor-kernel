---
id: 0087
title: Oracle waits are guest time, and a host that cannot host the question gets no verdict
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0008, 0022, 0064, 0066, 0068, 0070, 0074, 0076, 0079, 0081, 0083, 0088]
amended: 2026-08-14
---

# ADR-0087: oracle waits are guest time, and a starved host gets no verdict

## Acceptance status

**Accepted** (2026-08-10). Written after the repository owner asked for the CI
red to be fixed and the design completed, and delegated the call: the two
decisions below — what an oracle wait is counted in, and who is to blame when a
host could not host the question — were made on their behalf and are recorded
here rather than left implicit in a script.

Implemented in the same series: `src/bootstrap/demos.rs` (cross-core waits),
`scripts/boot/qemu-boot-check.sh` (measurement and verdict).
`scripts/lib/boot-oracle.sh` is **unchanged** — deliberately, see §5.

## Context

`make check`'s richest gate boots the kernel under QEMU and asserts a hundred
lines of a healthy run. It has always had a third verdict — `INDETERMINATE`,
exit 3 — for the case where the emulator was starved by its host, on the
argument that _an emulator can be starved by its host; silicon cannot_. That
verdict was wired to exactly one line, `timer: MISSED`.

Three things came out of a CI red that would not reproduce anywhere else
(`preempt-el1-cpu1: spinner exit timeout`, ADR-0079's oracle, green on this
workstation and stamped on silicon):

1. **The waits were counted in the wrong clock.** A CPU 0 task waiting for CPU
   1 to make progress gave it a budget of 4096 `yield_now()` calls. How many of
   this core's yields fit before the other core takes its next step is a
   property of the _host's_ scheduler: under TCG the vCPU threads are
   multiplexed, and CPU 0 can spin through thousands of cheap yields while CPU
   1's thread has not been picked once. The claim being tested — the spinner
   observes the stop word and exits — is real; the budget was not a bound on
   anything the kernel controls.

2. **The starvation measurement was wrong twice, and unmeasurable once.** It
   read `/proc/self/stat` fields 15 and 16 — the shell's own stime plus the
   children's utime — instead of cutime and cstime. Corrected, it still read
   0.10 s for a full 15-second boot on CI, because the workflow wraps
   `qemu-system-aarch64` in a script that `exec docker run`s an Arch container:
   the emulator is not this shell's child at all, and no sampling of a pid we
   own will ever see it.

3. **The bar was picked, not measured.** "Under one whole core it cannot be
   asked to meet a deadline" — while this laptop's build shim caps the slice at
   exactly 1.00, so every local deadline failure would have been excused.

## Decision

### 1. A cross-core oracle wait is bounded in guest time

Waits for _another core's_ progress are bounded by the guest's own tick counter
(`CROSS_CORE_WAIT_TICKS`, ten ticks ≈ 1 s of guest time), with a yield ceiling
underneath so a stopped tick counter is a failure rather than a hang. Ticks
mean the same thing on every host; yields do not.

Same-core waits stay counted in yields, and that is not an inconsistency: there
the budget measures the very core whose progress is in question. Bring-up waits
before the timer is live (`core1 alive`, `core1 ipi`, `core1 ran`) stay spin-
bounded for the same reason — there is no guest clock yet.

> **Amendment (2026-08-11, reconciliation per [ADR-0058](0058-adr-amendments-and-mutation-freshness.md)).**
> This decision was implemented in `demos.rs` — the oracle — because in
> 2026-08-10 the oracle was where cross-core waits lived. [ADR-0088](0088-product-home-cpu.md)
> then pinned a **product** agent on CPU 1, which made the loader's drain
> barrier (`ipc::yield_until_empty`, a wait on the console server) cross-core
> too, still counted in 64 yields. Measured at 3 failures in 6 runs of
> `product-boot-check` on a host capped near one core, always as
> `loader: chirp drain wait FAILED Timeout` — chirp is the CPU 1 agent, and
> beacon on CPU 0 never failed. The barrier is now bounded by
> `ipc::DRAIN_WAIT_TICKS` (ten ticks, the same guest-time bound `demos` uses)
> with a yield ceiling under it. §1 is unchanged; a product site it did not
> reach has been brought under it.
>
> Same series, same cause: `qemu-product-boot-check.sh` asserted beacon's two
> console bytes as the contiguous string `H!`, one line under a comment saying
> the bytes may interleave. With chirp on CPU 1 they do — `H?!` is a correct
> boot — so the assertion was a claim about the host's vCPU scheduling. It now
> asserts the two bytes **in order**, tolerating other agents' bytes between
> them. §2's rule is that no assertion may be attributable to the host; this
> extends it from *when a verdict is given* to *what a verdict asserts*.

What this buys, on the quota ladder that measured it: the oracle set used to
break below 0.37 of a core and now holds at 0.22. Half of what looked like the
emulator needing CPU was the oracles needing the host's scheduler to be fair.

### 2. Below the bar there is no verdict, for any assertion

On a host that did not give the emulator enough CPU, **no** assertion is
attributable to the kernel. A starved boot does not fail the rotation claim and
pass the rest; it does not get far enough, and which line goes missing first is
a property of the host.

So the QEMU runner's `fail` consults the measured share: below
`CORES_TO_BE_MEASURABLE` every failure is `INDETERMINATE` (exit 3, non-zero —
an unestablished claim is not a pass), above it every failure is a plain red.

The rejected alternative was a per-assertion split into "structural" and
"deadline" claims, with only the second class excusable. It was implemented,
and the quota ladder refuted it: every rung surfaced a new arrival-shaped
assertion the classification had missed — the `task-a`/`task-b` interleave,
then the agent's bytes arriving before its report, then the pl011 RX loopback,
then the received payload. They were never a class. They were the assertions
that happened to come first.

### 3. A run ends when the guest has said what the oracle needs

The boot window is a ceiling, not a duration. A fixed fifteen seconds is a
host-sensitive way to ask a fixed question, and a run that fell a second short
of the tail looked exactly like a kernel that never printed it. Each boot now
ends when the log carries the oracle's last stage **and** the steady-state soak
the old window bought (ten tick reports — the assertions about what must *not*
appear are only as strong as the time the kernel was left running, so the soak
is kept and expressed in the guest's clock rather than the host's).

### 4. The bar is measured, and the measurement says how it was taken

`CORES_TO_BE_MEASURABLE` is 0.40 cores. The ladder is in the script: clean at
2.14, frayed at 0.23 — where what breaks first is not an assertion timing out
but the serial console dropping a line mid-run — unusable at 0.13. The bar sits
clear of that edge rather than on it.

The share is read from the emulator's own `/proc/<pid>/stat` while it runs.
When the process this script started is not the emulator — `comm` says
`docker`, because CI's wrapper runs QEMU in a container — the runner falls back
to the host's `/proc/stat` busy delta and labels the number host-wide wherever
it prints it. The discriminator is the process's name, not the size of its
reading: that client still burns 0.06 s relaying serial, so "is the reading
zero?" answered no and an earlier version of this fallback never fired.

Never the other way round: on CI the host-wide reading is 2.1 cores per boot,
so a failure there is a plain red, as it should be.

Every verdict, red or indeterminate, prints the share it was decided on, and
the clean line prints one per boot.

### 5. Silicon has no verdict to soften

`scripts/lib/boot-oracle.sh` — the single definition of "this boot is sound",
shared verbatim by the QEMU gate and `hw-transcript-check.sh` — is untouched by
all of this. The assertions do not know which runner is asking. Only the QEMU
runner has a starved-host excuse to weigh; the board owns its four cores, so
its `fail` is unconditional.

### 6. The product gate uses the same host verdict

> **Amendment (2026-08-14, reconciliation per [ADR-0058](0058-adr-amendments-and-mutation-freshness.md)).**

The product image is a separate composition gate, but it is still a QEMU
execution whose serial assertions can be starved by the host. Its runner now
measures the same 0.40-core bar before calling `product-oracle.sh`: below the
bar it returns `INDETERMINATE` (exit 3), and above it `timer: MISSED` is passed
to the hard-failure callback used by the hardware transcript checker. The
product gate therefore cannot turn an unjudged or deadline-missing boot green
by omitting that line from its oracle.

## Consequences

- A deadline failure on a starved host is exit 3, not exit 1. `make check` and
  CI both still stop, and the operator is told which of the two happened.
- The gate is honest about CI's container wrapper instead of silently reading
  zero through it. If the fallback ever reads zero as well, the run says
  "not credible — treat as unmeasured" rather than inventing starvation.
- ADR-0079/0081/0083's claims are unchanged in content. What changed is the
  clock their waits are counted in, so the same claim now means the same thing
  on a workstation, in CI's container, and on the board.

## Gates

| Check       | Evidence                                                                                     |
| ----------- | -------------------------------------------------------------------------------------------- |
| QEMU        | `make boot-check` clean at 2.14 cores; `INDETERMINATE` (exit 3) at 0.22, 0.13 and 0.08     |
| Product QEMU | `make product-boot-check` reports `INDETERMINATE` at 0.03 cores; product assertions are not run |
| Measurement | Sampler agrees with `/usr/bin/time` on an identical run (1.89 vs 2.06 cores)                 |
| HW          | `hw-transcript-check.sh` unchanged and still sharing every assertion                         |

## Related

- Masked regions and the mask that travels: [0022](0022-blocking-recv-and-the-mask-that-travels.md)
- The oracles whose waits this re-clocks: [0079](0079-k8-per-core-timer-preemption-first-slice.md),
  [0081](0081-k8-el0-on-cpu1-first-slice.md), [0083](0083-k8-work-stealing-first-slice.md)
- Boot oracle and its two runners: [0066](0066-sd-media-durable-store.md)
