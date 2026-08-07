# Interrupts and exceptions (M1 + P0 idle/UART RX)

## Hardware evidence (Pi 4B Rev 1.5)

Re-verified after the move to an early MMU, which changed the memory regime the
polled gates run under — from no attributes to Normal WB with caches on:
`soft_ticks=3`, `gate: HPPIR=30 ok`, `inject: IAR=0x1e id=30`, `ticks 0 -> 2`,
`selftest: OK`. Build them with `--features bringup`; see
[`verification.md`](verification.md).

| Check                              | Result                  |
| ---------------------------------- | ----------------------- |
| PL011 console                      | OK                      |
| CNTP `ISTATUS` (IRQs masked)       | OK                      |
| GIC `HPPIR` = PPI 30 when pending  | OK                      |
| `GICC_IAR` claim + `EOIR`          | OK                      |
| Vector IRQ → `ticks=`              | **OK**                  |
| UART0 SPI 153 + RX ring + WFI idle | **OK** (HW, 2026-08-04) |

### Gotchas fixed during bring-up

1. **No `AtomicU64` before MMU** — exclusive load/store can livelock without
   proper memory attributes. _Historical, M1 only:_ this held while the MMU was
   off. Since M2 the RAM is Normal WB Inner-Shareable and atomics are the
   required way to share state with the IRQ path (architecture rule 7). Do not
   re-derive plain counters from this note.
2. **Do not reprogram CNTP while the timer line is live on the GIC** without
   masking the PPI first (observed hang).
3. **Group 0 + `IAR`/`EOIR`** works for this firmware/GIC path; relying only on
   Group 1 aliased registers was unreliable during early bring-up.
4. Prefer **`HPPIR`** over a zero **`AHPPIR`** reading when both are present.
5. **SPI must set `ITARGETSR` to CPU0** — PPIs are banked; UART SPI 153 will
   not reach core 0 without a target bit.

## Dispatch table

The table lives in [`kernel_core::irqtable`](../crates/kernel-core/src/irqtable.rs),
host-tested; `src/irq` owns the chip, the interrupt mask, the counters an
exception context can reach, and the call itself.

**Sealing is the load-bearing part.** The table is mutable during bring-up and
frozen by `irq::seal()` afterwards, and that is the whole reason the IRQ path
may hold a shared `&'static` borrow while an interrupt arrives: after the seal
there is no writer left to race. It was a rule nothing checked — `seal()` set a
flag, `register()` read it, and nothing had ever registered a handler after
sealing to watch it refuse.

`register` returns `Result`, not `bool`, because the two refusals need
different fixes: an id past the table is a constant to correct, a sealed table
is a bring-up ordering bug. `bsp::rpi4::irq::BindError` carries which, and that
error is what a refusal to boot prints.

A claimed interrupt gets one of three answers, and the last two are counted
apart on purpose:

| Answer                       | Meaning                     | Counter          |
| ---------------------------- | --------------------------- | ---------------- |
| `Handle { handler, cookie }` | call it                     | —                |
| `Unhandled`                  | in range, nobody registered | `irq: unhandled` |
| `OutOfRange`                 | beyond the table            | out-of-range     |

An in-range miss is a line someone enabled and forgot to claim; an id past the
table is a chip reporting something this kernel does not believe in. Collapsing
them would hide the second inside the first.

Bring-up prints `irq: sealed with N handlers registered` and `boot-check`
asserts it: a boot that registered nothing looks exactly like a healthy one
until the first interrupt nobody answers.

## Layering

```
exception_irq_el1
    → irq::handle_cpu_irq()
         chip.claim() → table.lookup(id) → handler(cookie) → chip.end()

exception_irq_el0        ← lower-EL IRQ during an unmasked EL0 session
    → save user context → El0Outcome::Irq
    → agent: irq::handle_cpu_irq() then el0::resume (re-execute)

time::on_timer_irq       ← TIMER_IRQ (PPI 30)
console::on_uart_rx_irq  ← UART_IRQ  (SPI 153) → RX ring only (when kernel owns drain)
arch/timer               ← CNTP only (no GIC)
drivers/gicv2            ← IrqChip (+ SPI target/level)
bsp/rpi4                 ← static GIC + bind
```

## Masking discipline: the scope, not just the state

Two different things mask interrupts here, and confusing them is how the one
real bug in this area was written.

**In EL0**, the mask is in the session's `SPSR`. `el0::set_entry_irqs_masked` /
`set_entry_irqs_unmasked` decide whether a lower-EL IRQ can take the vector at
all — that is the switch between "the agent runs uninterrupted" and
`El0Outcome::Irq` above.

**In EL1**, `cpu::without_irqs(f)` masks around a closure. It saves `DAIF`
before `f` and restores it after, and that pairing is the whole subtlety:

> **A `DAIF` save/restore pair must not span a call that can switch tasks.**

If a switch happens inside the closure, the saved value crosses into another
task's execution — the next task runs with this task's mask, and this task, when
it eventually resumes, restores a value captured in an epoch that has ended. It
is not a race to be closed by ordering; it is a scoping error.

The scheduler has always got this right by construction: `switch_with` does not
use `without_irqs` at all, it splits `irq_save` / `irq_restore` deliberately
around `context_switch`, because that call does not return on the caller's stack.

The EL0 session loop did **not**, and did not have to until
[ADR-0022](adr/0022-blocking-recv-and-the-mask-that-travels.md): it held one
mask for the whole session, which was sound only because nothing inside it ever
switched. `SYS_RECV` now parks the calling agent — a switch — so the masked
region shrank from the session to the step: `enter_step`, `resume_step`,
`end_step` in `src/agent/mod.rs` each take the mask, do the one thing
`arch::el0` requires it for, and give it back. The loop body between two steps
runs unmasked, and that is where the park happens.

`make irq-scope` (`scripts/check-irq-scope.sh`) keeps the rule true instead of
remembered. It walks each `cpu::without_irqs(` region by brace depth — a scope
is not a line, and the offending call is usually far below the one that opened
it — and fails on `block_current`, `yield_now`, `switch_with`, `context_switch`
or `sched::exit` inside one. `wake_task` is deliberately **not** on that list:
it makes a task ready and returns, and it takes `without_irqs` itself.

**What the gate does not see, stated rather than implied:** it is lexical.
`ipc::recv_from_slot` switches — it parks — but three frames down, so a call to
it inside a masked region passes. Catching that needs a call graph this tree
does not have. The direct form, which is how the mistake is actually written,
cannot land unnoticed; the indirect form is review's, and
[`verification.md`](verification.md) lists it among the gate blind spots.

## GIC-400 (BCM2711)

| Block | Base          |
| ----- | ------------- |
| GICD  | `0xFF84_1000` |
| GICC  | `0xFF84_2000` |

Requires `enable_gic=1` in `config.txt`.

## Timer

ARM Generic Timer physical: `CNTP_*`, IRQ **PPI 30**, frequency from
`CNTFRQ_EL0` (~54 MHz on the tested board).

## UART0 (P0 + agent RX own)

| Item          | Value                                                                                                               |
| ------------- | ------------------------------------------------------------------------------------------------------------------- |
| GIC id        | **SPI 153** (VC IRQ 57 + SPI base 96)                                                                               |
| PL011 sources | `RXIM` + `RTIM` (single-char with FIFO) when kernel owns RX                                                         |
| Handler       | `console::on_uart_rx_irq` → `ByteRing` (kernel drain)                                                               |
| Consumer      | idle (`pop_rx`) when drain live; EL0 poll of `DR` when agent owns RX                                                |
| Agent own     | `suspend_rx` masks IMSC then clears the base; LBE inject for self-test; `resume_rx` publishes the base then re-arms |

### Handover order

UART0 is level-triggered, so an armed line the handler has no view to drain
through cannot be cleared: the byte enters the handler, finds nothing to do,
returns without popping `DR` or writing `ICR`, and the line re-presents
immediately. One state — _armed, no view_ — and both handover orders once passed
through it.

The rules are [`kernel_core::rxline`](../crates/kernel-core/src/rxline.rs),
which decides and does not act: `suspend` and `resume` return the steps, and
`console` performs them. That is what lets a host test walk the line through
every intermediate state and ask, after each one, whether an interrupt arriving
at that instant could still be cleared — swapping either pair makes it red at
the exact step.

## Production bring-up

```
exception::init
bsp::irq::init(hz)       // timer + UART handlers, enable both lines
console::enable_rx_irq   // PL011 IMSC
timer::on_interrupt()    // clean deadline
irq_enable
idle console (drain ring, ticks, WFI)
```

Optional full gates: build with `--features bringup`. They are compiled out of a
production image entirely, along with the raw GIC accessors they need — there is
no longer a constant to flip.

## Console rule

| Context               | UART                                           |
| --------------------- | ---------------------------------------------- |
| bootstrap / main loop | exclusive `Pl011` **TX**; drain RX **ring**    |
| UART RX IRQ           | drain FIFO → ring only (**no** TX / `println`) |
| other IRQ handlers    | **no** UART                                    |
| panic                 | mask IRQ, `console::steal`, bounded TX         |
