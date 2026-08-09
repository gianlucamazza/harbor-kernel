# Technology stack

What Harbor is built **with**, and what is deliberately **not** in the box.
This page answers "what am I compiling, on what, and how is it checked" — it
does not own status (that is [`roadmap.md`](roadmap.md)) or the model (that is
[`architecture.md`](architecture.md)).

Short version in the [root README](../README.md#technology-stack).

---

## In one table

| Layer | Choice | Why / where |
| --- | --- | --- |
| Language | Rust, edition 2024, `no_std` | No runtime, no libc; the whole image is ours |
| Toolchain | **pinned** `1.96.0` + `rustfmt`, `clippy` | [`rust-toolchain.toml`](../rust-toolchain.toml) |
| Target | `aarch64-unknown-none-softfloat` | [ADR-0002](adr/0002-softfloat-kernel.md) |
| Board | Raspberry Pi 4 Model B (BCM2711), **single core** | [`hardware.md`](hardware.md), [ADR-0015](adr/0015-multi-arch-scaffold.md) |
| Entry | `kernel8.img` at `0x80000`, EL2 → EL1h | [`boot-chain.md`](boot-chain.md) |
| Dependencies | **none** — two workspace crates, zero external crates | [`Cargo.lock`](../Cargo.lock) |
| Build driver | `make` over `cargo` (+ `llvm-objcopy`) | [`Makefile`](../Makefile) |
| Execution model | Cooperative tasks; agent = EL1 driver task + EL0 program | [ADR-0006](adr/0006-cooperative-execution-model.md), [ADR-0023](adr/0023-an-agent-is-an-el1-driver-and-an-el0-program.md) |
| Authority model | Slot-indexed capabilities; manifest grants | [ADR-0017](adr/0017-el0-capability-abi.md), [ADR-0021](adr/0021-agents-as-data-and-the-manifest.md) |
| Verification | host tests · Miri · QEMU oracles · Pi 4B serial stamps | [`verification.md`](verification.md) |

---

## Runtime platform

| Fact | Value |
| --- | --- |
| SoC / board | BCM2711, Raspberry Pi 4 Model B (Rev 1.5 is the stamped unit) |
| ISA | AArch64 (Cortex-A72, ARMv8.0-A) |
| Cores used | **One.** SMP is an open track (**K8**), not a configuration switch |
| Exception levels | Firmware enters at EL2; `boot.s` drops to EL1h. EL3 is **refused**, not handled |
| Console | PL011 UART0 @ 115200, primary in every configuration |
| Interrupts | GICv2, Group 0 + `IAR`/`EOIR` ([ADR-0004](adr/0004-gic-group0-firmware-pin.md)) |
| Platform firmware | Closed VideoCore blobs, **pinned by hash** — never hidden ([`blobs.md`](blobs.md)) |

Board selection is compile-time (`board-*` feature) and the Makefile refuses any
`ARCH`/`BOARD` other than `aarch64`/`rpi4` rather than building something that
looks like it worked. The tree is multi-arch **ready**, not multi-arch product —
port checklist in [`porting.md`](porting.md), contract in
[`arch-contract.md`](arch-contract.md). A **lab** second ISA (QEMU x86 guest)
is intent-only — [ADR-0067](adr/0067-host-lab-second-isa-intent.md); no code
path until a boot gate exists. **Product and guest images are Linux-free**
(no Linux ABI or bootloader chain in the target path). The **dev host** that
runs `cargo` / `make` / QEMU may be Linux and is **not** part of the product
TCB — [native multi-arch practices](design/native-multiarch-practices.md).

## Language and target

- **`no_std`.** No `std`, no libc, no allocator crate: the heap is a free-list
  `GlobalAlloc` in `src/mm/`.
- **Softfloat.** The target is `…-none-softfloat` because `CPACR_EL1.FPEN` is
  never set: FP/SIMD traps instead of silently working. `make no-simd`
  disassembles the linked ELF and fails on any FP/SIMD register.
- **`panic = "abort"`** in both profiles. There is no unwinder to service
  `panic = "unwind"`; the handler in `src/panic.rs` masks IRQs, steals the
  console and parks.
- **Release profile** optimises for size (`opt-level = "s"`, LTO, one codegen
  unit) — the image is cold on every boot. Debug info and symbols are kept on
  purpose: two gates disassemble the ELF and need symbol names.
- **No atomic read-modify-write before the MMU is on** — an `LDXR`/`STXR` pair
  makes no progress on Device-nGnRnE memory on a Cortex-A72, and QEMU does not
  reproduce it. `make no-early-exclusives` guards the entry path.

## Workspace

| Crate | Contains | Rule it lives under |
| --- | --- | --- |
| `harbor-kernel` (root) | Kernel binary: MMIO, assembly, page tables, drivers, policy | `unsafe` allowed; every block needs a `SAFETY` comment (`undocumented_unsafe_blocks = "deny"`) |
| `crates/kernel-core` | Pure, total functions over integers: register encodings, allocator math, IPC and scheduler models | `unsafe_code = "deny"` (one scoped exception: the SPSC ring), **no dependencies**, runs on the host |

The split is what makes host testing possible at all: the kernel binary carries
its own `#[panic_handler]`, which collides with the test harness, so
`make test` builds `kernel-core` only. Module map:
[`docs/README.md` § Code layout](README.md#code-layout).

## Cargo features

| Feature | Default | What it buys |
| --- | --- | --- |
| `board-rpi4` | **on** | The active board package. Exactly one `board-*` per image; `--no-default-features` refuses with a named `compile_error!` |
| `oracle` | **on** | The demo tasks and agents every boot assertion reads. On by default because `make boot-check` *is* the oracle; `make product-builds` proves an image without it exists and carries no demo strings |
| `bringup` | off | Masked CNTP/HPPIR/IAR gates and raw GIC accessors used when the board will not talk |
| `debug-display` | off | Optional SPI TFT status panel (ILI9486 class) — observability, not agent capability ([ADR-0009](adr/0009-optional-spi-tft-debug-console.md)) |

Rule 9 of [`architecture.md`](architecture.md#rules): diagnostic scaffolding
lives behind a feature, never in the production surface.

## Host toolchain

| Tool | Needed for | Absent → |
| --- | --- | --- |
| `cargo` / `rustc` 1.96.0 | Everything | Hard failure (pinned) |
| `llvm-objcopy` | `kernel8.img` from the ELF | Hard failure |
| `llvm-objdump` | `make no-simd` | Gate **refuses to report clean** |
| `qemu-system-aarch64` (`raspi4b`) | `make qemu`, boot oracles | Named error |
| `python3` | Agent-store pack / inject / inspect ([ADR-0029](adr/0029-agent-store-in-image.md)) | Composition tooling unavailable |
| `shellcheck` | `make shellcheck` | **Skipped loudly** |
| Rust nightly + Miri | `make miri` | **Skipped loudly** |
| `cargo-mutants` | `make mutants` (not in `make check`) | Optional |
| 3.3 V USB-serial adapter | Hardware evidence | No `done (HW)` claim is possible |

A gate that passes because it did not run is the failure mode this project
treats as a bug: optional tools skip *with a message*, never silently.

## Verification stack

Four levels, in increasing cost and increasing authority
([`verification.md`](verification.md) is the index):

| Level | Mechanism | Claims it can settle |
| --- | --- | --- |
| Host | `make test` — unit, integration and doc-tests on `kernel-core` | Pure logic: encodings, allocator math, IPC/scheduler models |
| Host (aliasing) | `make miri` | The one `unsafe` in `kernel-core` — provenance and aliasing running the code cannot sample |
| Emulated | `make boot-check`, `make product-boot-check` | The kernel boots and reaches a healthy steady state; console oracles assert named lines. Reports **INDETERMINATE** rather than a red it cannot attribute |
| Silicon | Pi 4B serial transcript | Anything timing-, attribute- or firmware-dependent. QEMU has booted a kernel that hung on the board |

Structural gates (`layering`, `arch-board-free`, `irq-scope`, `no-static-mut`,
`no-simd`, `no-early-exclusives`, `board-guard`) and documentation gates
(`doc-claims`, `doc-symbols`, `xrefs`, `roadmap-evidence`) run in the same `make check`, which is a
deliberate superset of CI: a local green must predict a remote one.

**`done (QEMU)` and `done (HW)` are different words on purpose.** Status
vocabulary: [`docs/README.md`](README.md).

## Not in the stack

| Absent | Why |
| --- | --- |
| `std`, libc, POSIX, glibc | Different ABI and an ambient-authority world — permanently out of model |
| Third-party crates | Zero dependencies today; adding one is a boundary decision, not a convenience |
| Preemption, SMP, ASID residuals | **Open tracks** K4 IRQ preemption / K8 / K7 TTBR1·switch-cost (ASID first slice done HW) — [`roadmap.md`](roadmap.md) |
| A device tree parser | Board truth is compiled-in BSP constants; the DTB is mapped read-only for a future parser ([ADR-0011](adr/0011-dtb-mapped-board-constants-risk-accept.md)) |
| A filesystem, network or window stack | They belong **above** the kernel as agents, when a composition needs them (P2–P5) |
| An LLM / agent framework | "Agent" here is the isolation unit — [`glossary.md`](glossary.md) |

## Where the stack is decided

Changing anything on this page is a boundary move, and boundary moves get an
ADR first ([ADR-0001](adr/0001-multi-role-analysis.md)). The load-bearing ones:
[ADR-0002](adr/0002-softfloat-kernel.md) softfloat,
[ADR-0003](adr/0003-early-mmu.md) early MMU,
[ADR-0005](adr/0005-static-page-table-arena.md) page-table arena,
[ADR-0006](adr/0006-cooperative-execution-model.md) cooperative execution,
[ADR-0015](adr/0015-multi-arch-scaffold.md) arch/board facades,
[ADR-0019](adr/0019-no-static-mut.md) no `static mut`.
Full list: [`architecture.md` § Decisions and reviews](architecture.md#decisions-and-reviews).
