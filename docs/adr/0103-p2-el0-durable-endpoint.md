---
id: 0103
title: P2 durable storage endpoint for EL0 agents
status: accepted
date: 2026-08-13
accepted: 2026-08-13
related: [0017, 0021, 0036, 0045, 0056, 0099, 0102]
---

# ADR-0103: P2 durable storage endpoint for EL0 agents

## Acceptance status

**Accepted** (2026-08-13), on delegated authority. This is the next slice
named by the P2 roadmap after the EL1 durable backend and product name
binding.

## Context

The durable store already owns a bounded, media-backed region in EL1
([ADR-0045](0045-p2-durable-store.md), [ADR-0066](0066-sd-media-durable-store.md)).
The product could bind names, but no product agent could reach that store:
the existing put/get surface was trusted EL1 code only. Adding a syscall or a
shared user buffer would enlarge the kernel ABI and isolation surface.

## Decision

The product starts one EL1 `blob` service during authority assembly. It owns
the existing `durable` backend and exposes a one-message request/reply IPC
protocol:

- `blob` is a request SEND capability;
- `blob-reply` is a separate reply RECV capability;
- `PUT` carries a bounded key and payload; `GET` carries a bounded key;
- replies distinguish `OK`, `MISSING`, malformed requests, and backend errors.

The endpoint names and held positions are part of the composition vocabulary:
`blob = 1`, `blob-reply = 2`, after `console = 0`. The service keeps the
opposite channel ends and receives both through its private slots. Agents
never receive the durable-region address and no new syscall is introduced.

The first product composition includes a `blob` agent. It sends `cfg=persist`,
gets `cfg`, waits for the reply, and reports `S` through the console. The
product oracle requires the service-side `put ok` and `got` markers, the
separate capability bindings, and the agent's three successful sends. The
existing EL1 durable demo remains the negative/missing-key coverage.

## Consequences

The storage service is now a reusable composition boundary for later P3/P4
services. The wire payload is intentionally limited to seven bytes per field;
larger values require a later protocol successor rather than silent
truncation. A service task and two channels use additional capacity budget,
and the vocabulary must stay synchronized between Rust authority assembly and
the external store packer.

## Alternatives

| Option | Why not |
| --- | --- |
| Add `SYS_BLOB_PUT/GET` | Expands the syscall ABI and makes storage a kernel concern |
| Map the durable region into EL0 | Exposes serialization, media layout, and write authority |
| Give the agent one bidirectional capability | Request and reply rights become coupled; separate endpoints make direction explicit |
| Keep storage EL1-only | Leaves the P2 EL0 composition residual open |

## Evidence

| Level | What |
| --- | --- |
| Host | `kernel-core` blob protocol tests; bare-metal `cargo check` |
| Build | `make vocabulary-sync product-builds`; packer emits the five-agent store |
| QEMU | `product-boot-check` requires the service, both endpoint bindings, `blob: put ok`, and `blob: got` |
| HW | Same product lines on a Pi 4B, stamp 2026-08-14, transcript `20260814-113438.log` (`src=dcc997cc`): `authority: 1 blob ok`, `authority: 2 blob-reply ok`, `blob: put ok`, `blob: got`, `S`, `loader: blob ran sends=3 refusals=0`. `make hw-check` clean |
