# rpi_minimal_agentic — build and deploy
#
#   make            release kernel8.img
#   make debug      debug kernel8.img
#   make check      fmt + tests + no-SIMD + pre-MMU + QEMU boot + clippy
#   make test       host unit tests for the pure-logic crate
#   make miri       run those tests under Miri (nightly; checks the unsafe)
#   make bringup-builds  compile the --features bringup configuration
#   make fmt        rustfmt
#   make qemu       boot the image under QEMU (PL011 on stdio)
#   make qemu-gdb   same, halted, waiting for gdb on :1234
#   make blobs      fetch pinned platform firmware
#   make deploy     copy image + config + blobs to SD (SD_MOUNT=...)
#   make serial     open serial console (SERIAL_DEV=...)
#   make clean

TARGET      := aarch64-unknown-none-softfloat
PROFILE     ?= release
CARGO_OUT   := target/$(TARGET)/$(PROFILE)
ELF         := $(CARGO_OUT)/rpi_minimal_agentic
IMG         := $(CARGO_OUT)/kernel8.img

SD_MOUNT    ?= /run/media/$(USER)/boot
SERIAL_DEV  ?= /dev/ttyUSB0
BAUD        ?= 115200
OBJCOPY     ?= llvm-objcopy

# QEMU models the BCM2711 as `raspi4b`: PL011 UART0 is chardev serial0, so
# `-serial mon:stdio` lands on the same console the board prints to.
QEMU        ?= qemu-system-aarch64
QEMU_MACHINE ?= raspi4b
QEMU_FLAGS  ?= -M $(QEMU_MACHINE) -kernel $(IMG) -serial mon:stdio -display none

# Long enough for the boot assertions (two tick reports at 10 Hz) with margin.
BOOT_CHECK_SECONDS ?= 15

# Host tests cover the pure-logic crate only: the kernel binary carries its own
# `#[panic_handler]`, which collides with the one the test harness links in.
TEST_PKG    := kernel-core
HOST_TARGET ?= $(shell rustc -vV | sed -n 's/^host: //p')

CARGO_FLAGS := --target $(TARGET)
ifeq ($(PROFILE),release)
  CARGO_FLAGS += --release
endif

.PHONY: all debug img elf check test miri bringup-builds no-simd no-early-exclusives boot-check doc-claims fmt fmt-check qemu qemu-gdb blobs deploy serial clean

all: img

debug:
	$(MAKE) img PROFILE=debug

# Always invoke cargo; it decides whether work is needed.
elf:
	cargo build $(CARGO_FLAGS)

img: elf
	$(OBJCOPY) -O binary $(ELF) $(IMG)
	@echo "built $(IMG)"
	@ls -la $(IMG)

# Deliberately a superset of what CI runs: a green here has to predict a green
# there, or it is not worth running locally. Every CI job has a target here —
# including `miri`, which skips loudly when nightly is absent rather than
# letting the claim quietly become false.
check: fmt-check test no-simd no-early-exclusives boot-check bringup-builds miri doc-claims
	cargo clippy --target $(TARGET) -- -D warnings
	cargo clippy -p $(TEST_PKG) --target $(HOST_TARGET) -- -D warnings

# The README's two machine-checkable claims: the gate list and the test count.
# Both have drifted, the gate list twice — once on the commit that added a gate.
doc-claims:
	./scripts/check-doc-claims.sh

# Boot the image under QEMU and assert it reaches a healthy steady state.
# The assertions live in the script, not here and not in the CI workflow, so
# the two cannot drift apart.
boot-check: img
	./scripts/qemu-boot-check.sh $(IMG) $(BOOT_CHECK_SECONDS)

test:
	cargo test -p $(TEST_PKG) --target $(HOST_TARGET)

# The kernel is built softfloat: no FP/SIMD register may appear in the image.
# A silent switch back to a NEON-enabled target would otherwise only show up as
# a synchronous exception on the board, since CPACR_EL1.FPEN is never set.
no-simd: elf
	@! llvm-objdump -d --no-show-raw-insn $(ELF) \
	  | grep -oE '\b[qv][0-9]+(\.[0-9]+[bhsd])?\b' \
	  | head -5 | grep . \
	  || { echo "error: FP/SIMD registers found in $(ELF)" >&2; exit 1; }
	@echo "no-simd: clean"

# Nothing may use an atomic read-modify-write before the MMU is on: with
# translation off every access is Device-nGnRnE, where the LDXR/STXR pair makes
# no forward progress on Cortex-A72 — a silent hang no emulator reproduces.
# The script checks the whole entry path, not one function, and fails if the
# path grows.
no-early-exclusives: elf
	./scripts/check-pre-mmu-path.sh $(ELF)

# Miri interprets the host tests and checks the aliasing and provenance rules
# that running the code cannot sample. It covers the only `unsafe` in
# kernel-core: the SPSC ring's `UnsafeCell` buffer and its `Sync` assertion.
#
# Not part of `make check`: it needs nightly, and the toolchain pin is
# deliberately stable. Run it when touching the ring or the allocator.
miri:
	@if ! rustup toolchain list | grep -q nightly; then \
	  echo "miri: SKIPPED — nightly not installed (rustup toolchain install nightly --component miri)" >&2; \
	  exit 0; \
	fi; \
	cargo +nightly miri test -p $(TEST_PKG) --target $(HOST_TARGET)

# The bring-up gates are what you reach for when the board will not talk, which
# is the worst moment to discover they no longer compile. Nothing else builds
# this configuration, so nothing else would notice.
bringup-builds:
	cargo build $(CARGO_FLAGS) --features bringup
	cargo clippy --target $(TARGET) --features bringup -- -D warnings
	@echo "bringup-builds: clean"

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

blobs:
	./scripts/fetch-blobs.sh

deploy: img
	./scripts/deploy-sd.sh "$(SD_MOUNT)" "$(IMG)"

serial:
	./scripts/serial.sh "$(SERIAL_DEV)" "$(BAUD)"

clean:
	cargo clean
