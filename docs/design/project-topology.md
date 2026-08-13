# Project topology — scale axes

**Status:** design contract for *where code and docs grow*.  
**Not:** a completion claim or a product multi-arch announcement.

**Related:** [architecture.md](../architecture.md) (layering),
[porting.md](../porting.md), [progressive-isa-practices.md](progressive-isa-practices.md),
[native-multiarch-practices.md](native-multiarch-practices.md),
[ADR-0015](../adr/0015-multi-arch-scaffold.md).

Harbor scales by **orthogonal axes**, not by a single mega-crate or runtime
`dyn Arch`. New work must land on the right axis so the next ISA, board, or
lab slice does not fork the tree.

---

## Axes (normative)

```text
                    ┌─────────────────────┐
                    │  Plane A: runtime   │  freestanding image (Linux-free)
                    └──────────▲──────────┘
           ┌───────────────────┼───────────────────┐
           │                   │                   │
    ┌──────┴──────┐     ┌──────┴──────┐     ┌──────┴──────┐
    │ ISA         │     │ Board       │     │ Maturity    │
    │ arch/<isa>  │     │ bsp/<board> │     │ product|lab │
    │ target_arch │     │ board-*     │     │ entry path  │
    └─────────────┘     └─────────────┘     └─────────────┘
           │                   │                   │
           └───────────────────┼───────────────────┘
                               │
                    ┌──────────┴──────────┐
                    │  Policy / drivers   │  only after ISA+board bind
                    │  (cfg by arch need) │
                    └──────────▲──────────┘
                               │
                    ┌──────────┴──────────┐
                    │  kernel-core        │  pure, host-tested (no MMIO)
                    └─────────────────────┘
```

| Axis | Selects with | Lives in | Grows when |
| ---- | ------------ | -------- | ---------- |
| **ISA** | `target_arch` | `src/arch/<isa>/` | New CPU/boot/vectors |
| **Board** | exactly one `board-*` feature | `src/bsp/<board>/` | New SoC/QEMU machine bind |
| **Maturity** | entry + which modules compile | product path vs `src/lab/` | L0 → lab kernel → product |
| **Protocol** | driver modules | `src/drivers/` | New silicon protocol (not board) |
| **Policy** | product modules | `src/{sched,ipc,agent,…}` | Completeness tracks K/P |
| **Pure arithmetic** | always (host tests) | `crates/kernel-core/` | Decode, tables, models |
| **Evidence** | gates / stamps | `scripts/`, `docs/verification.md` | New oracle lines |

---

## Tree map (growth destinations)

```text
harbor-kernel/
├── crates/kernel-core/     # Axis: pure — NEVER MMIO/asm; host-tested
├── src/
│   ├── main.rs             # kernel_main only: product | lab dispatch
│   ├── arch/<isa>/         # Axis: ISA — boot.s, link.ld, contract roles
│   ├── bsp/<board>/        # Axis: board — memmap, console bind, irq ids
│   ├── drivers/            # Axis: protocol — board-agnostic
│   ├── lab/                # Axis: maturity lab — thin bring-up (not Pi stubs)
│   │   ├── mod.rs
│   │   ├── x86.rs          # H3 L0 entry (ADR-0071)
│   │   └── panic.rs        # lab panic bind (COM1); product keeps src/panic.rs
│   ├── bootstrap/ …        # Axis: product policy (aarch64 image)
│   └── …
├── scripts/
│   ├── check/              # invariant gates (always-on scale: add, don’t fork)
│   ├── boot/               # product + lab oracles (qemu-boot-check, qemu-x86-*)
│   ├── agent/              # composition tools
│   ├── host/               # SD / serial / blobs
│   └── lib/                # shared shell
├── docs/
│   ├── architecture.md     # normative model today
│   ├── design/             # contracts (topology, multi-arch, progressive ISA)
│   ├── adr/                # immutable decisions
│   └── verification.md     # evidence index
└── Makefile                # PRODUCT | LAB | HOST sections
```

---

## Maturity paths (do not merge casually)

| Path | Entry | Modules compiled | Gate |
| ---- | ----- | ---------------- | ---- |
| **Product** | `bootstrap::run` | Full aarch64 policy + `board-rpi4` | `make boot-check` / HW stamp |
| **P3 composition** | `bootstrap::run` | AArch64 QEMU `virt` + modern virtio-mmio transport and descriptor lifecycle | `make qemu-virtio-check` |
| **Lab** | `lab::<isa>::run` | Minimal: arch + bsp + drivers needed + `lab/` | `make x86-boot-check` (today) |

**Rule:** lab never “stubs in” the product tree so it compiles. Product modules
stay `cfg`’d out on lab targets ([progressive-isa P.3](progressive-isa-practices.md)).

When lab gains sched/timer, grow **inside** `lab/` or graduate shared policy
into modules that both paths can `cfg` in — do not copy `bootstrap` wholesale.

---

## Where to put a new change (decision table)

| You are adding… | Put it here | Do **not** |
| --- | --- | --- |
| CPU register op / trap / switch | `src/arch/<isa>/` | BSP or driver |
| Port/MMIO base, pinmux, IRQ id | `src/bsp/<board>/` | `arch/` (arch-board-free) |
| UART/GIC/APIC protocol | `src/drivers/` | BSP (BSP only binds) |
| Family/model decode, pure math | `crates/kernel-core/` | `lab/` or BSP |
| L0/L1 lab bring-up step | `src/lab/` | Fake product bootstrap |
| Completeness track feature | product policy + ADR + roadmap | Lab-only file without gate |
| New boot oracle | `scripts/boot/` + Makefile | Inline in CONTRIBUTING only |
| Structural boundary | `docs/adr/` first | Drive-by rename of axes |

---

## Makefile scale (three bands)

| Band | Targets | May open when |
| ---- | ------- | ------------- |
| **PRODUCT** | `img`, `boot-check`, `deploy`, `ARCH`/`BOARD` allowlist | Product combo only (today aarch64/rpi4) |
| **LAB** | `x86-elf`, `x86-boot-check`, `qemu-x86` | Lab gate exists (`done (QEMU-x86)` …) |
| **HOST** | `test`, `miri`, `fmt`, `layering`, `doc-*` | Always; no guest required |

Do **not** overload `ARCH=x86_64` for product until a successor to ADR-0007
claims multi-product. Lab stays dedicated targets.

---

## Workspace scale (when to add a crate)

| Add a crate when… | Keep in `harbor-kernel` when… |
| ----------------- | ----------------------------- |
| Logic is pure, host-tested, no `unsafe` MMIO | It needs `arch`/`bsp`/asm |
| Shared by multiple future binaries | It is one image’s policy |
| Extraction shrinks `kernel-core` *coherence* cost | Split would force circular deps |

Default: **one binary crate + kernel-core**. Do not create `harbor-x86` as a
second package until packaging needs force it (P.9: packaging ≠ ABI).

---

## Docs scale

| Fact | Owner (one place) |
| ---- | ----------------- |
| Layering rules | `architecture.md` |
| Where files go / axes | **this page** |
| Port checklist | `porting.md` |
| Progressive ISA honesty | `progressive-isa-practices.md` |
| Linux-free + support bar | `native-multiarch-practices.md` |
| Status of tracks | `roadmap.md` |
| Evidence | `verification.md` |

---

## Checklist for a scale-safe PR

- [ ] Change sits on one primary axis (table above).  
- [ ] No new import edge that `make layering` forbids.  
- [ ] Lab path did not gain product stubs; product path did not gain lab-only IO.  
- [ ] New gate or oracle if a new “done” claim is made.  
- [ ] Docs owner updated (not a third copy of status).  
- [ ] `docs/README.md` code layout names new top-level modules.
