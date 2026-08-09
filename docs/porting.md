# Porting guide (ISA and board)

Harbor is **multi-arch ready** (ADR-0015) with **one product target**:
AArch64 + Raspberry Pi 4 Model B. This page is the checklist for a future port.
It does not claim a second architecture is supported.

Contract of the arch facade: [`arch-contract.md`](arch-contract.md).
Layering rules: [`architecture.md`](architecture.md).

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

## Add an ISA

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
4. Extend `scripts/check/layering.sh` if the facade-isolation regex must name
   the new ISA directory.
5. Toolchain / build:
   - `rust-toolchain.toml` `targets`
   - `.cargo/config.toml` per-target `rustflags` (linker script path)
   - `build.rs` `rerun-if-changed` for the new boot/link/vectors
   - Makefile `TARGET` / `ARCH` allowlist
6. Provide a board that uses the ISA (`board-*` + memmap for that SoC).
7. CI: add a job only when there is a real boot gate (QEMU machine or hardware).
   Do not add a compile-only skeleton target that bitrots.
8. Update `docs/arch-contract.md` if the facade surface grows; file an ADR if
   the user/kernel separation model changes (e.g. leaving TTBR0-only).
9. Teach the platform self-check the new core (ADR-0065): add its row to
   `kernel_core::cpuid::part`, revisit the boot refusals for what is
   load-bearing on that ISA, and give the boot oracle the `cpu:` identity the
   new runner is expected to report.

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
