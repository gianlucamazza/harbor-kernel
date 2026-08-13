# Harbor (harbor-kernel) — build and deploy
#
# Scale bands (docs/design/project-topology.md):
#   PRODUCT — aarch64/rpi4 image, boot-check, deploy
#   LAB     — dedicated x86-* targets (not ARCH=); ADR-0071
#   HOST    — test, miri, fmt, layering, doc gates
#
#   make                 PRODUCT: release kernel8.img
#   make check           HOST gates + PRODUCT boot/product-boot/oracle-census (+ clippy)
#   make test / miri     HOST: kernel-core
#   make boot-check      PRODUCT QEMU oracle (full demo fleet)
#   make product-boot-check / oracle-census   PRODUCT composition minimum + MAX_TASKS ratchet
#   make x86-elf / x86-boot-check / qemu-x86   LAB (ADR-0071)
#   make deploy / serial / blobs               PRODUCT board ops
#   make clean
#
# Product ARCH/BOARD allowlist (ADR-0015): refusal, not multi-select.
# Lab stays outside this allowlist until a multi-product ADR.
SUPPORTED_ARCH  := aarch64
SUPPORTED_BOARD := rpi4
ARCH        ?= $(SUPPORTED_ARCH)
BOARD       ?= $(SUPPORTED_BOARD)
ifneq ($(ARCH),$(SUPPORTED_ARCH))
$(error unsupported ARCH=$(ARCH); only $(SUPPORTED_ARCH) is product-supported — see docs/porting.md)
endif
ifneq ($(BOARD),$(SUPPORTED_BOARD))
$(error unsupported BOARD=$(BOARD); only $(SUPPORTED_BOARD) is product-supported — see docs/porting.md)
endif

TARGET      := aarch64-unknown-none-softfloat
PROFILE     ?= release
CARGO_OUT   := target/$(TARGET)/$(PROFILE)
ELF         := $(CARGO_OUT)/harbor-kernel
IMG         := $(CARGO_OUT)/kernel8.img

# H3 L0 lab (ADR-0071) — freestanding ELF for QEMU -kernel + PVH note.
X86_TARGET  := x86_64-unknown-none
X86_OUT     := target/$(X86_TARGET)/$(PROFILE)
X86_ELF     := $(X86_OUT)/harbor-x86.elf
X86_CARGO_FLAGS := --target $(X86_TARGET) --no-default-features --features board-qemu-q35
ifeq ($(PROFILE),release)
  X86_CARGO_FLAGS += --release
endif

SD_MOUNT    ?= /run/media/$(USER)/boot
SERIAL_DEV  ?= /dev/ttyUSB0
BAUD        ?= 115200
OBJCOPY     ?= llvm-objcopy
# Parametric so the "refusing to report clean" branch of `no-simd` is reachable
# in a test, and so a distro that ships versioned LLVM binaries can point at one.
OBJDUMP     ?= llvm-objdump

# QEMU models the BCM2711 as `raspi4b`: PL011 UART0 is chardev serial0, so
# `-serial mon:stdio` lands on the same console the board prints to.
QEMU        ?= qemu-system-aarch64
QEMU_MACHINE ?= raspi4b
# ADR-0029: packed store for inject into `.agent_store` (product image).
AGENTS_BIN  ?= target/agents.bin
# raspi4b: min 4 CPUs (QEMU). -smp 4 so ADR-0070 can unpark core 1.
QEMU_FLAGS  ?= -M $(QEMU_MACHINE) -smp 4 -kernel $(IMG) -serial mon:stdio -display none

# Lab x86 (ADR-0071): q35 + COM1 16550. Not product.
QEMU_X86         ?= qemu-system-x86_64
QEMU_X86_MACHINE ?= q35
QEMU_X86_CPU     ?= qemu64
QEMU_X86_FLAGS   ?= -machine $(QEMU_X86_MACHINE) -cpu $(QEMU_X86_CPU) -m 128M \
	-kernel $(X86_ELF) -serial mon:stdio -display none -no-reboot

# A **ceiling**, not a duration (ADR-0087): the boot check stops as soon as the
# guest has printed everything the oracle needs, so a fast host still finishes
# in about ten seconds and a slow one is not cut off mid-oracle. 15 was the old
# fixed window and was, on CI, occasionally a second short of the tail.
BOOT_CHECK_SECONDS ?= 45
# Lab L0 is banner + cpu + halt. Slightly longer than bare minimum so a
# busy host still reaches long mode before `timeout` fires (seen empty
# serial at load ≥15 with a 3s budget).
X86_BOOT_CHECK_SECONDS ?= 5

# Host tests cover the pure-logic crate only: the kernel binary carries its own
# `#[panic_handler]`, which collides with the one the test harness links in.
TEST_PKG    := kernel-core
HOST_TARGET ?= $(shell rustc -vV | sed -n 's/^host: //p')
ifeq ($(strip $(HOST_TARGET)),)
$(error could not determine the host triple from 'rustc -vV' — is rustc on PATH?)
endif

# Optional cargo features for img/deploy (e.g. FEATURES=bringup).
# Default images stay featureless so QEMU boot-check and production match.
FEATURES    ?=

CARGO_FLAGS := --target $(TARGET)
ifeq ($(PROFILE),release)
  CARGO_FLAGS += --release
endif
ifneq ($(strip $(FEATURES)),)
  CARGO_FLAGS += --features $(FEATURES)
endif

.PHONY: all debug img elf check test miri bringup-builds \
	debug-builds board-guard product-builds shellcheck xrefs doc-symbols no-simd \
	no-early-exclusives no-static-mut irq-scope \
	boot-check panic-check hw-check mutation-freshness x86-elf x86-boot-check doc-claims layering fmt fmt-check \
	qemu qemu-gdb qemu-virtio-check qemu-x86 blobs deploy \
	restore-rpios serial clean agents vocabulary-sync

all: img

debug:
	$(MAKE) img PROFILE=debug

# Always invoke cargo; it decides whether work is needed.
elf:
	cargo build $(CARGO_FLAGS)

# FEATURES stamp: empty is the product image. Every image is headless since
# ADR-0094 retired the panel; the stamp stays because `bringup` and
# `panic-probe` are still opt-in, and a deploy that silently shipped one of
# those would be the same surprise the glass build used to be.
img: elf
	$(OBJCOPY) -O binary $(ELF) $(IMG)
	@echo "built $(IMG)"
	@if [ -z "$(strip $(FEATURES))" ]; then \
	  echo "  FEATURES=(none)  — product image"; \
	else \
	  echo "  FEATURES=$(FEATURES)"; \
	fi
	@ls -la $(IMG)

# A superset of CI's *gates*: a green here has to predict a green there, or it
# is not worth running locally. The one CI step with no prerequisite here is
# `make blobs` — a network fetch of pinned firmware, deliberately kept out of a
# target people run offline. Everything else CI runs has a target below, and
# `miri`/`shellcheck` fail loudly when their tool is absent rather than letting
# the claim quietly become false (skip only with ALLOW_MIRI_SKIP=1 /
# ALLOW_SHELLCHECK_SKIP=1, same shape as boot-check's ALLOW_BOOT_SKIP).
check: fmt-check test no-simd no-early-exclusives no-static-mut irq-scope boot-check panic-check bringup-builds debug-builds board-guard product-builds product-boot-check oracle-census miri mutation-freshness doc-claims doc-symbols layering arch-board-free shellcheck xrefs roadmap-evidence vocabulary-sync
	cargo clippy --target $(TARGET) -- -D warnings
# `--all-targets` so the host tests are linted too. Without it `make check` was
# no longer a superset of CI, which is the one property this target claims: CI
# grew a clippy pass over test code and found an orphaned doc comment that no
# local run could have seen.
	cargo clippy -p $(TEST_PKG) --target $(HOST_TARGET) --all-targets -- -D warnings

# Every gate in this Makefile is a shell script, and two of them carry
# `# shellcheck source=` directives — the intent was always there, the check was
# not. `-x` follows the sourced library, `-P scripts` so it can find it.
# Skipped loudly rather than silently when shellcheck is absent: a linter that
# passes because it did not run is the failure `no-simd` was fixed for.
shellcheck:
	@if ! command -v shellcheck >/dev/null; then \
	  if [ "$${ALLOW_SHELLCHECK_SKIP:-}" = "1" ] && [ "$${CI:-}" != "true" ]; then \
	    echo "shellcheck: SKIPPED — not installed (ALLOW_SHELLCHECK_SKIP=1)" >&2; \
	    exit 0; \
	  fi; \
	  if [ "$${CI:-}" = "true" ]; then \
	    echo "shellcheck: FAIL — not installed, and a skip is refused in CI (ADR-0096)" >&2; \
	    exit 1; \
	  fi; \
	  echo "shellcheck: FAIL — not installed; refusing to report clean" >&2; \
	  echo "  install it (pacman -S shellcheck) or set ALLOW_SHELLCHECK_SKIP=1" >&2; \
	  exit 1; \
	fi; \
	shellcheck -x -P scripts scripts/check/*.sh scripts/boot/*.sh scripts/host/*.sh scripts/lib/*.sh && echo "shellcheck: clean"

# Facts that live in two places with nothing comparing them: markdown links,
# `ADR-NNNN` citations, and the status and id each ADR repeats in the index.
# All four were true when the gate was written — by attention, which does not
# survive a rename.
xrefs:
	./scripts/check/xrefs.sh

# A module path in the descriptive docs must name code that is there. `xrefs`
# follows links and `doc-claims` counts two numbers; neither sees a sentence
# that names `arch::mmu::EARLY_L1` after the map moved to `src/mm/early.rs`.
# Path-aware on purpose: the symbol still exists, in another module.
doc-symbols:
	./scripts/check/doc-symbols.sh

# ADR-0099: the composition's vocabulary is stated in two files — the kernel
# declares the indices, the packer writes them into store slots. A disagreement
# grants an agent authority the composition did not choose, silently, so the two
# copies are compared rather than trusted.
vocabulary-sync:
	./scripts/check/vocabulary-sync.sh

# The layering rules in docs/architecture.md, checked against real imports.
# They are the architecture, and were enforced by review alone until now.
layering:
	./scripts/check/layering.sh

# `layering` sees imports; this sees the other way of knowing the board, which
# is to write its addresses out by hand. F23 lived in that blind spot.
arch-board-free:
	./scripts/check/arch-board-free.sh

# The README claims a machine can settle, plus the arch facade against its contract.
# Both have drifted, the gate list twice — once on the commit that added a gate.
doc-claims:
	./scripts/check/doc-claims.sh

# Boot the image under QEMU and assert it reaches a healthy steady state.
# The assertions live in scripts/lib/boot-oracle.sh (shared with the HW
# transcript check), not here and not in the CI workflow, so no copy drifts.
# ADR-0029: pack composition blob (input to scripts/agent/inject-store.py).
agents:
	python3 scripts/agent/pack-store.py -o $(AGENTS_BIN)

boot-check: img
	./scripts/boot/qemu-boot-check.sh $(IMG) $(BOOT_CHECK_SECONDS)

# P3 transport gate: build the dedicated AArch64 QEMU virt composition and
# prove modern virtio-mmio negotiation plus refusal when the device is absent.
# Persistent IRQ service, packet I/O, and EL0 capability evidence are
# deliberately separate successors; this target must not make those claims by
# sharing the product oracle.
QEMU_VIRT_OUT := target/$(TARGET)/$(PROFILE)
QEMU_VIRT_ELF := $(QEMU_VIRT_OUT)/harbor-kernel
QEMU_VIRT_IMG := $(QEMU_VIRT_OUT)/harbor-qemu-virt.img

qemu-virtio-check:
	cargo build --target $(TARGET) --no-default-features --features board-qemu-virt,oracle $(if $(filter release,$(PROFILE)),--release,)
	$(OBJCOPY) -O binary $(QEMU_VIRT_ELF) $(QEMU_VIRT_IMG)
	./scripts/boot/qemu-virtio-boot-check.sh $(QEMU_VIRT_IMG) $(BOOT_CHECK_SECONDS)

# The panic path is the one product path whose evidence was entirely negative
# — every gate above asserts that no boot printed PANIC (ADR-0093). This boots
# an image that faults on purpose, in its own runner: `boot-oracle.sh` fails on
# the banner, and relaxing that would blunt the gate that matters every day.
panic-check:
	cargo build $(CARGO_FLAGS) --features panic-probe
	$(OBJCOPY) -O binary $(CARGO_OUT)/harbor-kernel $(CARGO_OUT)/kernel8-panic.img
	./scripts/boot/qemu-panic-boot-check.sh $(CARGO_OUT)/kernel8-panic.img

# --- LAB band (project-topology; not product ARCH=) -------------------------
# H3 L0 (ADR-0071): freestanding x86_64 ELF for QEMU -kernel (PVH note).
# Packaging: cargo bin is `harbor-kernel`; publish as `harbor-x86.elf`.
x86-elf:
	cargo build $(X86_CARGO_FLAGS)
	cp -f $(X86_OUT)/harbor-kernel $(X86_ELF)
	@echo "built $(X86_ELF)"
	@ls -la $(X86_ELF)

x86-boot-check: x86-elf
	./scripts/boot/qemu-x86-boot-check.sh $(X86_ELF) $(X86_BOOT_CHECK_SECONDS)

test:
	cargo test -p $(TEST_PKG) --target $(HOST_TARGET)

# The kernel is built softfloat: no FP/SIMD register may appear in the image.
# A silent switch back to a NEON-enabled target would otherwise only show up as
# a synchronous exception on the board, since CPACR_EL1.FPEN is never set.
# The register set is the whole point, so the pattern covers the scalar FP
# registers (d/s/h) as well as the vector ones (q/v): a non-softfloat target
# emits `fmov d0, …` and `scvtf s1, w0` long before it reaches a `v` register,
# and the earlier pattern would have watched those go past. `x`/`w` are the
# general-purpose registers and are of course everywhere.
#
# Data directives are dropped before matching. `objdump` prints the raw bytes of
# a literal pool even under `--no-show-raw-insn`, and a byte that happens to be
# `d0`..`d9` reads as the register `d0`. A `ldr x0, =0x30d00800` in `boot.s` is
# what found this: the gate went red on a change that emits no FP at all. The
# earlier `[qv]` pattern could not hit it, because `q` and `v` are not hex
# digits — widening the pattern is what made the disassembly's data sections
# start to matter.
#
# The tool check is not decoration. Without it a missing `llvm-objdump` makes
# the pipeline produce nothing, `grep .` fail, `!` invert that into success,
# and the target print `no-simd: clean` having disassembled nothing at all —
# the exact failure `scripts/check/pre-mmu-path.sh` refuses by design.
no-simd: elf
	@command -v $(OBJDUMP) >/dev/null || { \
	  echo "no-simd: FAIL — $(OBJDUMP) not found; refusing to report clean" >&2; \
	  echo "  install it (pacman -S llvm) — this gate inspects the linked ELF" >&2; \
	  exit 1; }
	@! $(OBJDUMP) -d --no-show-raw-insn $(ELF) \
	  | grep -vE '^\s*[0-9a-f]+:.*\.(word|byte|short|long)\b' \
	  | grep -oE '\b([qv][0-9]+(\.[0-9]+[bhsd])?|[dsh][0-9]+)\b' \
	  | head -5 | grep . \
	  || { echo "error: FP/SIMD registers found in $(ELF)" >&2; exit 1; }
	@echo "no-simd: clean"

# Nothing may use an atomic read-modify-write before the MMU is on: with
# translation off every access is Device-nGnRnE, where the LDXR/STXR pair makes
# no forward progress on Cortex-A72 — a silent hang no emulator reproduces.
# The script checks the whole entry path, not one function, and fails if the
# path grows.
no-early-exclusives: elf
	./scripts/check/pre-mmu-path.sh $(ELF)

# No `static mut` under src/ (ADR-0019). Rule 7 is absolute after CURRENT_EL0
# became an AtomicPtr; this gate is what keeps a second exception from landing
# as a comment nobody re-checks. Does not need the ELF — it greps source.
no-static-mut:
	./scripts/check/no-static-mut.sh

# ADR-0022: a DAIF save/restore pair must not span a call that can switch tasks.
# Brace-aware rather than line-based — a scope is not a line, and the offending
# call is usually far below the `without_irqs(` that opens the region.
irq-scope:
	./scripts/check/irq-scope.sh

# Rule 9: diagnostic scaffolding stays out of the production surface. Builds the
# image without the `oracle` feature and refuses one that still carries the demo
# strings — derived from `demos.rs`, not listed here, because a hand-written
# marker list passed a real leak twice while this was being written.
product-builds:
	./scripts/boot/product-image.sh

# M8: product image (no oracle) must actually run beacon + console server.
# Composition-minimum QEMU smoke (excellence F-R5-2): not a second oracle.
product-boot-check: product-builds
	./scripts/boot/qemu-product-boot-check.sh

# ADR-0085 / multi-role F-R7-1: MAX_TASKS is oracle tax, not density.
# Source, architecture table, and documented last raise must agree.
#
# ADR-0098: the fourth number — what the product actually occupies — is booted
# and read off the invariant beacon's `slots=` field rather than carried as a
# constant. The target therefore boots the product image (a few seconds), and
# refuses to fall back to a remembered peak when the field is missing.
oracle-census:
	./scripts/check/oracle-census.sh

# Miri interprets the host tests and checks the aliasing and provenance rules
# that running the code cannot sample. It covers the only `unsafe` in
# kernel-core: the SPSC ring's `UnsafeCell` buffer and its `Sync` assertion.
#
# A `make check` prerequisite (nightly is required only for this target; the
# kernel's own toolchain pin stays stable). ALLOW_MIRI_SKIP=1 opts out loudly.
# The mutation run's stamp against the surface `cargo mutants --list` reports
# today (~2s). Fails when the scope gained or lost mutable surface since the
# last run — the freshness half of ADR-0058 that had no gate until ADR-0096,
# and that K8 landed `done (HW)` straight through.
mutation-freshness:
	./scripts/check/mutation-freshness.sh

# Assert a Pi 4B serial transcript against the boot oracle.
#
# The one gate that sees what QEMU cannot: TLB fills are speculative on
# silicon and not in TCG, so the ADR-0050 staleness class is only ever red
# here. It had no target until 2026-08-11 and was invoked from memory — which
# is a gate nobody runs on the day it matters.
#
#   make hw-check TRANSCRIPT=.serial-log/20260810-160227.log
hw-check:
	@if [ -z "$(strip $(TRANSCRIPT))" ]; then \
	  echo "hw-check: TRANSCRIPT=<serial-capture log> is required" >&2; \
	  echo "  e.g. make hw-check TRANSCRIPT=.serial-log/20260810-160227.log" >&2; \
	  exit 1; \
	fi
	./scripts/check/hw-transcript-check.sh "$(TRANSCRIPT)"

# Mutation testing. Not a `check` prerequisite: ~7 minutes, and the value is in
# reading which mutants survived rather than in a threshold. Cadence and scope
# rules are ADR-0058's: run before a boundary-moving commit, and the script
# refuses an artifact that did not cover its own file list. `make check` runs
# `mutation-freshness`, which fails when the mutable surface has moved since
# the last recorded run.
mutants:
	./scripts/host/run-mutants.sh

# Roadmap status flips must leave a trace in the evidence index.
roadmap-evidence:
	./scripts/check/roadmap-evidence.sh

miri:
	@if ! rustup toolchain list | grep -q nightly; then \
	  if [ "$${CI:-}" = "true" ]; then \
	    echo "miri: FAIL — nightly not installed, and a skip is refused in CI (ADR-0096)" >&2; \
	    exit 1; \
	  fi; \
	  if [ "$${ALLOW_MIRI_SKIP:-}" = "1" ]; then \
	    echo "miri: SKIPPED — nightly not installed (ALLOW_MIRI_SKIP=1)" >&2; \
	    exit 0; \
	  fi; \
	  echo "miri: FAIL — nightly not installed; refusing to report clean" >&2; \
	  echo "  rustup toolchain install nightly --component miri, or set ALLOW_MIRI_SKIP=1" >&2; \
	  exit 1; \
	fi; \
	cargo +nightly miri test -p $(TEST_PKG) --target $(HOST_TARGET)

# The bring-up gates are what you reach for when the board will not talk, which
# is the worst moment to discover they no longer compile. Nothing else builds
# this configuration, so nothing else would notice.
bringup-builds:
	cargo build $(CARGO_FLAGS) --features bringup
	cargo clippy --target $(TARGET) --features bringup -- -D warnings
	@echo "bringup-builds: clean"

# ADR-0009 SPI/status path must keep compiling even when the default image is
# headless — same rationale as bringup-builds.

# `make debug` is what someone reaches for with gdb, and the dev profile has a
# different opt-level and different codegen from the one every other gate
# builds. Nothing else compiles it, so nothing else would notice it breaking.
debug-builds:
	cargo build --target $(TARGET)
	cargo clippy --target $(TARGET) -- -D warnings
	@echo "debug-builds: clean"

# ADR-0015 puts board selection behind a feature and backs it with a
# `compile_error!`. An error message is a claim like any other: this asserts the
# build fails, and fails *saying why*, rather than with a cascade about a
# missing `bsp::board`.
board-guard:
	@if cargo build --target $(TARGET) --no-default-features 2>&1 \
	   | grep -q 'no board selected'; then \
	  echo "board-guard: clean (refused with the intended message)"; \
	else \
	  echo "board-guard: FAIL — building with --no-default-features did not refuse" >&2; \
	  echo "  expected the compile_error! in src/bsp/mod.rs to name the missing feature" >&2; \
	  exit 1; \
	fi

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

# Emulated boot. Exit with Ctrl-A x (the monitor is multiplexed onto stdio).
qemu: img
	@command -v $(QEMU) >/dev/null || { \
	  echo "error: $(QEMU) not found (pacman -S qemu-system-aarch64)" >&2; exit 1; }
	$(QEMU) $(QEMU_FLAGS)

# Halted at reset, waiting for: gdb -ex 'target remote :1234' $(ELF)
qemu-gdb: img
	@command -v $(QEMU) >/dev/null || { \
	  echo "error: $(QEMU) not found (pacman -S qemu-system-aarch64)" >&2; exit 1; }
	$(QEMU) $(QEMU_FLAGS) -S -s

# H3 L0 lab guest (ADR-0071). Exit with Ctrl-A x.
qemu-x86: x86-elf
	@command -v $(QEMU_X86) >/dev/null || { \
	  echo "error: $(QEMU_X86) not found (pacman -S qemu-system-x86)" >&2; exit 1; }
	$(QEMU_X86) $(QEMU_X86_FLAGS)

blobs:
	./scripts/host/fetch-blobs.sh

deploy: img
	@if [ -z "$(strip $(FEATURES))" ]; then \
	  echo "deploy: FEATURES=(none) — product image."; \
	else \
	  echo "deploy: FEATURES=$(FEATURES)"; \
	fi
	./scripts/host/deploy-sd.sh "$(SD_MOUNT)" "$(IMG)"

# Same mount-point guard as deploy; needs a prior backup under .sd-backup/.
restore-rpios:
	./scripts/host/restore-rpios-boot.sh "$(SD_MOUNT)"

serial:
	./scripts/host/serial.sh "$(SERIAL_DEV)" "$(BAUD)"

# Non-interactive half of `serial`: timestamps every line and opens a FIFO so a
# script can send bytes to the board. Used for anything that has to answer
# "how long between X and Y", which a picocom transcript cannot.
serial-capture:
	./scripts/host/serial-capture.sh

clean:
	cargo clean
