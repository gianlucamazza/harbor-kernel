# rpi_minimal_agentic — build and deploy
#
#   make            release kernel8.img
#   make debug      debug kernel8.img
#   make check      cargo check + clippy (-D warnings)
#   make fmt        rustfmt
#   make blobs      fetch pinned platform firmware
#   make deploy     copy image + config + blobs to SD (SD_MOUNT=...)
#   make serial     open serial console (SERIAL_DEV=...)
#   make clean

TARGET      := aarch64-unknown-none
PROFILE     ?= release
CARGO_OUT   := target/$(TARGET)/$(PROFILE)
ELF         := $(CARGO_OUT)/rpi_minimal_agentic
IMG         := $(CARGO_OUT)/kernel8.img

SD_MOUNT    ?= /run/media/$(USER)/boot
SERIAL_DEV  ?= /dev/ttyUSB0
BAUD        ?= 115200
OBJCOPY     ?= llvm-objcopy

CARGO_FLAGS := --target $(TARGET)
ifeq ($(PROFILE),release)
  CARGO_FLAGS += --release
endif

.PHONY: all debug img elf check fmt blobs deploy serial clean

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

check:
	cargo check --target $(TARGET)
	cargo clippy --target $(TARGET) -- -D warnings

fmt:
	cargo fmt --all

blobs:
	./scripts/fetch-blobs.sh

deploy: img
	./scripts/deploy-sd.sh "$(SD_MOUNT)" "$(IMG)"

serial:
	./scripts/serial.sh "$(SERIAL_DEV)" "$(BAUD)"

clean:
	cargo clean
