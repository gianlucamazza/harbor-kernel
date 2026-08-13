---
id: 0101
title: The first composed driver-agent — a device the store grants
status: accepted
date: 2026-08-12
accepted: 2026-08-12
related: [0013, 0021, 0027, 0029, 0034, 0049, 0087, 0088, 0096, 0099, 0100]
---

# ADR-0101: The first composed driver-agent — a device the store grants

## Acceptance status

**Accepted** (2026-08-12), on delegated authority. The owner chose the window
this product declares — **RNG200** — and delegated the rest of the design.
[ADR-0099](0099-composition-vocabulary.md) and
[ADR-0100](0100-device-windows.md) were accepted the same way; this is the
slice that turns their vocabulary into a sentence.

## Context

[ADR-0100](0100-device-windows.md) built the mechanism and declared no window
with it. Every entry naming one is refused by arithmetic, which is a property
worth having and is not yet a product: no composition grants a device, so the
claim that a driver-agent can **arrive** rather than be **compiled in** has no
instance.

The driver-agents in the tree are still the compiled kind — `demos.rs` calls
`map_device_page(USER_RNG_VA, RNG200_BASE, …)` with the base from the BSP, and
that agent exists only in an image built to contain it.

## Decision

The product declares one window, **`rng` at index 0**, and an agent in the
store asks for it. That agent is not a demo: it is packed by
`pack-store.py`, injected into `.agent_store`, and granted its page by the same
index arithmetic that grants the console.

### 1. A window is provided only if the board has the device

`bootstrap::run` already probes the RNG200 and prints one line — `rng200: ok
word=…` or `rng200: unavailable (NotPresent)`. That probe now also answers a
question: `probe_rng` returns whether the device is there, and `authority`
provides the window only when it is.

The probe is not repeated. Initialising a device twice to answer the same
question is how two answers start to disagree.

### 2. An absent device is not a failed one

[ADR-0099](0099-composition-vocabulary.md) made a vacancy in the capability
vocabulary a **failure**: a service that should have started did not, and
`product-boot-check` refuses `VACANT`. A device window cannot take that rule.
QEMU's `raspi4b` has no RNG200, and `rng200: unavailable (NotPresent)` there is
a correct boot of a correct kernel on a board without that block.

So the window vocabulary distinguishes what the capability one does not need to:

| Line                        | Meaning                                                         |
| --------------------------- | --------------------------------------------------------------- |
| `authority: 0 rng ok`       | Declared, probed, provided. An agent naming it gets the page    |
| `authority: 0 rng absent`   | Declared; this board does not have the device. **Not an error** |
| `authority: 0 rng FAILED e` | Declared, the board should have had it, and providing it failed |

`product-boot-check` accepts `ok` **or** `absent` and refuses `FAILED` — and it
does not choose between the first two from a list of boards. It reads the
`rng200:` line already in the transcript and requires the two to agree. One
oracle, two boards, and the expectation derived from what the board said rather
than from what a script remembers (the technique `product-image.sh` uses for
its marker list, and `oracle-census` for `MAX_TASKS`).

### 3. The agent proves it read the device, not that it was mapped

A program that is handed a page and exits proves the mapping did not fault. It
does not prove it read anything: an encoder that dropped the load would pass
that test.

So `entropy` — the agent — reads `RNG_CTRL` (offset 0) from its window, isolates
`CTRL_RBGEN`, and sends `R` to the console endpoint if the bit is set, `r` if it
is not. The kernel's own probe enables the block, so on a board with an RNG200
the answer is `R`, and it is an answer only a program that actually loaded from
the device page can give.

New encoder: `prog::encode_read_device_bit_console_exit(va_hi16, reg_off,
bit, console_slot, byte_hi, byte_lo)`. Same shape as
`encode_pl011_rx_poll_exit`, which already reads a device register and sends a
byte — this one branches on a bit rather than on FIFO emptiness.

### 4. The composition names the window by name, and the packer resolves it

`pack-store.py` grows `--window <name>` and `--device-va <addr>`, resolving the
name through its `WINDOWS` table — the table `make vocabulary-sync` already
compares against `authority.rs`. A composition is written in names; the wire
carries the index; the two are kept honest by a gate that exists.

## Alternatives

| Option                                              | Why not                                                                                                                                                                                                                                    |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Declare a window present on QEMU (a timer register) | The positive path would be green in `make check` every day, which is worth a lot — but the first _device_ an agent drives should be a device, not a counter chosen for the emulator's convenience. The owner chose RNG200 knowing the cost |
| Declare the PL011                                   | It is the console's own device. Granting it in the product fights the console server and ADR's RX-masking rule, and the conflict would be the story rather than the mechanism                                                              |
| Keep the agent in the oracle manifest               | Then it is compiled in again, which is the thing this ADR exists to stop being true                                                                                                                                                        |
| Treat `absent` as a failure, and gate on the board  | Two oracles, one per board, disagreeing on the day it matters — the shape `hw-transcript-check` exists to avoid                                                                                                                            |

## Consequences

- **The positive path is not green in `make check`.** QEMU has no RNG200, so on
  the emulator `entropy` is refused for a vacant window, and the only evidence
  that a composed driver-agent works is a hardware stamp. This is stated rather
  than worked around: it is the cost of the owner's choice, and the gate says
  `absent` out loud instead of passing quietly.
- The refusal path is exercised daily and by two different vacancies: `nowindow`
  (index past the vocabulary) and `entropy` on QEMU (declared, absent).
- The product store grows a third agent, so `slots=` moves and `oracle-census`
  will read a new peak.
- `Perms::USER_RO` gets its first user: the RNG window is granted **read-only**.
  An agent that can read entropy has no business writing the control register,
  and the vocabulary is where that is now expressible (ADR-0100 §2).

## The gate that would catch this ADR's reversal

`product-boot-check` requires the `authority: 0 rng …` line to agree with the
`rng200:` line, so a window declared and silently never provided fails, and so
does a product that stops declaring it. On a board with the device it further
requires `entropy` to have run and `R` to have reached the wire — the byte that
only a real load can produce.

Going back to a compiled-in driver-agent removes the store entry, and the store
assertions fail before the device ones do.

## Evidence

| Level | What                                                                                                                                                                              |
| ----- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Host  | The encoder's bytes against the assembler oracle; `bind_window` resolving the entry against a provided and an absent window (ADR-0100's tests, now with a real caller)            |
| QEMU  | `authority: 0 rng absent` agreeing with `rng200: unavailable`, `entropy` refused for a vacant window, no page mapped, and the rest of the composition unaffected                  |
| HW    | Pi 4B stamp: `rng200: ok`, `authority: 0 rng ok`, `loader: entropy loaded`, `R` on the wire, `entropy ran refusals=0` — a driver-agent that arrived in a store and drove a device |
