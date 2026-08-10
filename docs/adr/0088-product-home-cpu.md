---
id: 0088
title: Product multi-core — manifest home_cpu and loader pin
status: accepted
date: 2026-08-10
accepted: 2026-08-10
related: [0021, 0023, 0027, 0029, 0070, 0075, 0076, 0080, 0081, 0082, 0083]
---

# ADR-0088: Product `home_cpu` (composition-visible affinity)

## Acceptance status

**Accepted** (2026-08-10). Design **and** first code slice for multi-role
Tier C residual: dual-current mechanism is **done (HW)**
([ADR-0070](0070-k8-smp-first-slice.md)…[0083](0083-k8-work-stealing-first-slice.md));
product agents still defaulted to CPU 0 with no store field. This ADR names
affinity on the **manifest / agent store** and has the loader admit with
sticky home.

Status after land: **done (QEMU)** via `make product-boot-check` (and host
tests); HW stamp optional follow-up on Pi when convenient.

## Context

| Exists | Gap |
| --- | --- |
| `sched::spawn_on` sticky home | Loader always `spawn_with_slots` → admit CPU 0 |
| EL0-on-CPU1 lab ([ADR-0080](0080-k8-el0-on-cpu1-design.md)/[0081](0081-k8-el0-on-cpu1-first-slice.md)) | Oracle-only `spawn_on(1, …)` |
| Architecture / SECURITY product home-CPU prose | No composition surface |

Home is a property of the **EL1 driver task** ([ADR-0023](0023-an-agent-is-an-el1-driver-and-an-el0-program.md)),
not of the EL0 image bytes.

## Decision

### 1. Field

| Item | Choice |
| --- | --- |
| Name | `home_cpu: u8` on `kernel_core::manifest::AgentEntry` |
| Domain | `0 .. N_CPUS` (`N_CPUS = 2` today) |
| Default | **0** when absent (builtin table; store reserved zero) |
| Refuse | `home_cpu >= N_CPUS` at parse / load — not a silent wrap |

### 2. Wire format (ADR-0027 store)

Version stays **1**. The existing per-agent **reserved `u32`** (after slots)
carries:

| Bits | Meaning |
| --- | --- |
| `7:0` | `home_cpu` |
| `31:8` | must be zero (else `ParseError::BadHome`) |

Old blobs with reserved `0` parse as home **0** — no version bump, no
re-pack of historical stores required for the default path.

### 3. Loader / sched

| Rule | Detail |
| --- | --- |
| Admit | `spawn_with_slots_on(home_cpu, agent_body, &slots)` |
| CPU1 online | Unchanged: secondary must be online ([ADR-0076](0076-k8-per-core-queues-first-slice.md)); else spawn fails loud |
| Steal | Agents still mark non-stealeable ([ADR-0082](0082-k8-work-stealing-design.md)); home is sticky |
| Console TX | Product `kprintln` / agent console path uses locked TX ([ADR-0077](0077-smp-shared-state-discipline.md)); EL0 `SYS_SEND` is the agent print path on any home |
| Builtin table | All entries `home_cpu = 0` |

### 4. Product composition (first evidence)

Default pack (`scripts/agent/pack-store.py`):

| Agent | home_cpu |
| --- | --- |
| beacon | 0 |
| chirp | **1** |

Beacon stays on the product home core; chirp proves a **store-pinned**
product agent on CPU 1 without oracle demos. Absent-field default remains 0
for any third agent / single-beacon pack.

### 5. Evidence

| Gate | Assertion |
| --- | --- |
| Host | parse/pack round-trip `home_cpu`; refuse out-of-range |
| `product-boot-check` | `loader: chirp loaded` … `home=1`; beacon `home=0`; chirp/beacon ran + wire bytes |
| Log shape | `loader: {name} loaded text=… stack=… home={n}` |

## Non-goals

- Ambient “spread agents across cores”
- Agent+TLB steal product default
- Cores 2–3, P3/P4
- Raising `MAX_TASKS` for dual-home demos

## Alternatives rejected

| Option | Why not |
| --- | --- |
| VERSION 2 only | Breaks inject of existing blobs for zero gain; reserved field is free |
| EL1-only API, no store field | Does not close composition surface (issue #24) |
| Soft default “any idle core” | Contradicts sticky home + SECURITY product pin honesty |

## Follow-up

- HW stamp on Pi when convenient (same product inject path as QEMU).
- Close GitHub [#24](https://github.com/gianlucamazza/harbor-kernel/issues/24) when QEMU green and docs flipped.
