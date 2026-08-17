---
id: 0108
title: Link acquisition in the boot path
status: proposed
date: 2026-08-17
related: [0095, 0105, 0106, 0107]
---

# ADR-0108: Link acquisition in the boot path

## Status

**Proposed.** This ADR decides how the Pi 4 boot path may wait for Ethernet
link, and what it prints while it waits. It does not claim a NIC and does not
move any status.

## Context

`Genet::init_phy` identifies the PHY, issues a bounded BMCR reset, and then
demands link-up:

```rust
// src/drivers/genet.rs:730-735
pub fn init_phy(&self) -> Result<PhyLink, Error> {
    let identified = self.identify_phy()?;
    self.reset_phy()?;
    let bmsr = self.mdio_read(phy::BMSR)?;
    identified.with_bmsr(bmsr).require_up().map_err(Error::Phy)
}
```

`reset_phy` (`:712-725`) waits only for `BMCR_RESET` to self-clear. That bit
clears in microseconds. A BMCR reset returns the PHY to defaults and restarts
autonegotiation, and 1000BASE-T autoneg takes **seconds**. The BMSR read that
follows therefore samples a link that cannot yet be up, and `require_up()`
fails by construction.

This is not a hypothesis. It is what the 2026-08-16 14:39 stamp
(`src=3f2d01b8`, transcript `20260816-052739.log`) printed: `genet: phy init
(bmcr, not a nic)`, then `link=down`, then `tx unavailable (link down)` and
`rx unavailable (link down)`. Before that slice the same boot reached the
doorbell and printed `tx cons len=124`. The slice that added `init_phy` did
not reveal a fault — it introduced one.

Linux never does this. `bcmgenet_open` hands the PHY to `bcmgenet_mii_probe`
(`bcmgenet.c:3403`) and then to `phy_start` (`:3348`); link-up arrives later,
as a link event, and `bcmgenet_mii_setup` reprograms UniMAC speed when it
does. Nothing in the open path blocks on link.

Harbor cannot copy that shape as-is: it has no PHY state machine, no link
interrupt wired, and — the constraint that actually decides this ADR — the
GENET bring-up runs **inside the boot path**, where an unbounded wait is a
hang, and a multi-second bounded wait is a boot that visibly stalls
([ADR-0095](0095-boot-phases.md): each phase reads what an earlier phase
produced; none of them is allowed to become a timeout).

Until this is resolved, no other GENET change can be measured: the doorbell is
never rung, so every boot reports the same refusal regardless of what else
changed ([ADR-0107](0107-genet-sequence-first-bring-up.md) §5).

## Decision

### 1. The boot path does not reset the PHY

`init_phy`'s BMCR reset leaves the boot path. Identification (`identify_phy`)
and classification (`classify_link`) stay: they are reads, they are bounded in
microseconds, and they are the two facts the report line needs.

Rationale: a boot-path PHY reset buys nothing Harbor currently uses. It does
not configure RGMII delays (the DT says `rgmii-rxid`; the PHY owns the delay),
it does not select a speed Harbor can act on before autoneg completes, and its
only observable effect so far has been to destroy the link state the firmware
had already established. The Pi firmware brings the PHY up for network boot;
inheriting that link is legitimate, and inheriting it is what the pre-`init_phy`
boots did when they reached the doorbell.

### 2. Link is a classified fact, not a precondition the boot path creates

`LinkState::Down` at boot is a **legitimate outcome**, not a failure. The
product prints its classification and refuses TX/RX with the existing
`tx/rx unavailable (link down)` lines. That refusal is correct behaviour and
stays.

What changes is that the boot path stops _causing_ the down state and then
reporting it.

### 3. A bounded settle is permitted, and it is bounded by a number

If a future slice needs to wait for link — for example because the firmware
did not leave one up — it may wait, subject to all of:

| Rule           | Value                                                                                                                                                             |
| -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| What is polled | `BMSR_LINK` **and** `BMSR_ANEG_DONE`, not `BMCR_RESET`                                                                                                            |
| Bound          | a wall-clock bound expressed in `CNTFRQ_EL0` ticks, **not** a spin count. `poll::until(RESET_SPIN_LIMIT, …)` is a spin budget and says nothing about elapsed time |
| Ceiling        | **≤ 250 ms** on the boot path. Longer belongs to a link-event slice, not to boot                                                                                  |
| Report         | the wait prints its own line, so a slow boot is explained rather than mysterious; a timeout is `link=down`, never a panic and never a silent retry                |
| Refusal        | a timed-out wait leaves the phase unchanged: still `Enabled`, still refusing at the doorbell                                                                      |

A wait that cannot state its ceiling in milliseconds does not go on the boot
path.

### 4. Waiting for a real link belongs to a later slice, outside boot

Acquiring a link that the firmware did not leave up — autoneg restart,
link-event interrupt on INTRL2_0, UniMAC speed reprogrammed when the event
arrives — is the shape Linux uses and the shape Harbor will need for a
published network service. It is **not** boot-path work, and it is not in
scope here. When it is written it needs its own ADR, because it introduces the
first asynchronous device event that changes UniMAC configuration after boot.

### 5. `PhyInitReport` survives as a name

The report type stays so the vocabulary does not churn; what it reports becomes
identification plus classification rather than a reset. A future reset slice,
if one is ever justified, reuses it.

## Consequences

### Positive

- The boot path stops guaranteeing its own refusal, so
  [ADR-0107](0107-genet-sequence-first-bring-up.md)'s sequence group becomes
  measurable in a single boot.
- The pre-`init_phy` behaviour that reached the doorbell is recovered without
  reverting a commit: it is restated as a decision, with the reason written
  down.
- "How long may a boot phase wait" gets a number instead of a habit, which is
  reusable beyond GENET.

### Negative / costs

- Harbor inherits a link it did not establish. That is honest but it means the
  first successful TX, when it happens, is evidence about the _controller_, not
  about Harbor's PHY bring-up. The ADR-0105 gate already says this: it asks for
  probe, link state, one TX, one RX, recovery and refusal — not for Harbor to
  have negotiated the link itself.
- If the firmware leaves the link down, the boot path reports down and the gate
  is not reachable that session. Mitigation is §3's bounded settle, and beyond
  it §4's link-event slice.

## The gate that catches its own reversal

`make hw-check` against the transcript: the boot must print the `PhyIdentify`
line and a `LinkReport` line, and must not print a `PhyInitReport::Reset`. A
reintroduced boot-path BMCR reset shows up as the reset line returning, and as
`tx unavailable (link down)` replacing whatever the doorbell reported — which
is precisely the diff this ADR exists to prevent, recorded on 2026-08-16 in
[`../verification.md`](../verification.md).
