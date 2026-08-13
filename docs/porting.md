# Porting guide (ISA and board)

Harbor is **multi-arch ready** (ADR-0015) with **one product target**:
AArch64 + Raspberry Pi 4 Model B. This page is the checklist for a future port.
It does not claim a second architecture is supported.

**Lab second ISA (L0 paid):** QEMU x86_64 bare guest — first code slice
[ADR-0071](adr/0071-h3-l0-x86-qemu-first-slice.md) (`make x86-boot-check`,
**done (QEMU-x86)**); intent/non-goals [ADR-0067](adr/0067-host-lab-second-isa-intent.md);
role matrix
[`design/host-lab-platform-matrix.md`](design/host-lab-platform-matrix.md).
Product `make` remains aarch64/rpi4 only; lab uses dedicated `x86-*` targets.

**Native support bar + Linux-independence:** full checklist in
[`design/native-multiarch-practices.md`](design/native-multiarch-practices.md).
Short form:

| Bar | Rule |
| --- | --- |
| Supported | Boot gate green for that ISA×board; not compile-only |
| Guest path | Linux-free (no Linux ABI, GRUB/EFI-stub, in-tree Linux drivers) |
| Lab x86 boot | QEMU `-kernel` freestanding ELF + **PVH** note (ADR-0071); not Multiboot1 |
| Dev host | May be Linux; **non-TCB** — must not leak into product assumptions |
| Evidence | `done (QEMU-x86)` ≠ `done (QEMU)` ≠ `done (HW)` |

Contract of the arch facade: [`arch-contract.md`](arch-contract.md).
Layering rules: [`architecture.md`](architecture.md).
**Where files live as the tree grows:**
[`design/project-topology.md`](design/project-topology.md).

## Add a board (same ISA)

1. Create `src/bsp/<board>/` with `memmap`, `console`, `irq`, and any bind-only
   modules (mirror `rpi4/`).
2. Add Cargo feature `board-<board>` in the root `Cargo.toml`.
3. Wire `src/bsp/mod.rs`:
   - `#[cfg(feature = "board-<board>")] pub mod <board>;`
   - `board` re-export under the same feature
   - Prefer exactly one `board-*` enabled (add mutual-exclusion `compile_error!`
     when a second board exists).
4. Keep drivers board-agnostic (bases/IRQ ids from BSP only).
5. If the board needs different silicon drivers, add them under `drivers/` and
   bind from the BSP — do not implement protocols in the BSP.
6. Update Makefile `BOARD` allowlist if you use `make BOARD=…`.
7. Document QEMU machine / hardware path in `docs/hardware.md` (or board note).
8. Run `make layering` and a board-appropriate boot gate.

Default remains `board-rpi4`. Building with `--no-default-features` and no
`board-*` must fail with the `bsp` `compile_error!`.

The P3 transport composition is built explicitly with
`--no-default-features --features board-qemu-virt,oracle` and verified with
`make qemu-virtio-check`. That target uses QEMU `virt`, GICv2, one AArch64
CPU, and a modern virtio-mmio net device. It proves transport and bounded
split-ring descriptor lifecycle only, and
reset/absence behavior; queue DMA, packet I/O, and EL0 capabilities require
their own evidence before being marked complete.

## Add an ISA

Follow the **scale axes** in
[`design/project-topology.md`](design/project-topology.md): ISA first, then
board, then maturity (lab vs product). Progressive honesty:
[`design/progressive-isa-practices.md`](design/progressive-isa-practices.md).

1. Implement `src/arch/<isa>/` providing every module in
   [`arch-contract.md`](arch-contract.md).
2. Place `boot.s` (or equivalent), `link.ld`, and exception vectors under that
   tree; include boot from `<isa>/mod.rs`.
3. Wire `src/arch/mod.rs`:
   ```rust
   #[cfg(target_arch = "<isa>")]
   mod <isa>;
   #[cfg(target_arch = "<isa>")]
   pub use <isa>::{ /* same facade set */ };
   ```
   Adjust the `compile_error!` for unsupported arches.
4. **Lab first (recommended):** `src/lab/<isa>.rs` + `lab/mod.rs` export +
   dedicated `make <isa>-boot-check` — do **not** stub product `bootstrap` on
   the new target.
5. Provide a board (`board-*` + `src/bsp/<board>/`).
6. Toolchain / build:
   - `rust-toolchain.toml` `targets`
   - `.cargo/config.toml` per-target `rustflags` (linker script path)
   - `build.rs` `rerun-if-changed` for the new boot/link/vectors
   - Makefile **LAB** band targets (not product `ARCH=` until multi-product ADR)
7. CI: add a job only when there is a real boot gate. No compile-only skeletons.
8. Update `docs/arch-contract.md` if the facade surface grows; file an ADR if
   the user/kernel separation model changes.
9. Platform self-check (ADR-0065): pure decode in `kernel_core::cpuid`, arch
   only reads registers; oracle asserts the `cpu:` line.

## What not to do

- Import `crate::arch::aarch64` or `crate::bsp::rpi4` from policy (`make layering` fails).
- Put board MMIO bases in `arch`.
- Implement UART/GIC protocol logic in the BSP.
- Introduce `dyn Arch` for the whole CPU surface without a new ADR.
- Claim multi-arch _support_ in README while only one target boots.

## Verification after any port work

```text
make layering
make check          # product combo: aarch64 + board-rpi4
```

For a new combo, define an equivalent boot-check before calling it supported.
