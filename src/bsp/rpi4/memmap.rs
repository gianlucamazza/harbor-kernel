//! BCM2711 physical memory map (Raspberry Pi 4 Model B).
//!
//! Peripheral window base is `0xFE00_0000` (Pi 4). Pi 2/3 used `0x3F00_0000`;
//! those constants must not be reused here.

/// Start of the low peripheral MMIO window.
pub const PERIPHERAL_BASE: usize = 0xFE00_0000;

/// GPIO controller.
pub const GPIO_BASE: usize = PERIPHERAL_BASE + 0x0020_0000;

/// PL011 UART0.
pub const UART0_BASE: usize = PERIPHERAL_BASE + 0x0020_1000;

/// RNG200 (iproc-rng200) register block — 0x28 bytes.
///
/// Low peripheral mode: legacy bus `0x7E10_4000` → ARM `0xFE10_4000`.
/// Covered by the existing peripherals Device window (no extra map).
pub const RNG200_BASE: usize = PERIPHERAL_BASE + 0x0010_4000;

/// SPI0 (SPI master 0) register block.
///
/// BCM2711 low peripheral window; same layout as the BCM2835 SPI0 block.
#[cfg(feature = "debug-display")]
pub const SPI0_BASE: usize = PERIPHERAL_BASE + 0x0020_4000;

/// Core clock used as the SPI0 source when `core_freq_min=500` in `config.txt`.
///
/// The SPI bit rate is `SPI0_CORE_CLOCK_HZ / CDIV`. If the firmware core clock
/// changes, re-measure before claiming a panel Fmax margin.
#[cfg(feature = "debug-display")]
pub const SPI0_CORE_CLOCK_HZ: u32 = 500_000_000;

/// SPI bit-clock ceiling for Waveshare-class ILI9486 (Hz).
///
/// Closed on silicon at 8 MHz with regwidth-16 framing (2026-08-05). Raise
/// toward 16 MHz only after re-checking colour bars / status on glass.
#[cfg(feature = "debug-display")]
pub const SPI0_TARGET_HZ: u32 = 8_000_000;

/// UART reference clock after platform firmware enables the PL011 (Hz).
///
/// Requires `enable_uart=1` in the boot partition `config.txt`.
pub const UART0_CLOCK_HZ: u32 = 48_000_000;

/// Console baud rate.
pub const UART0_BAUD: u32 = 115_200;

/// GIC-400 distributor (GICD).
pub const GICD_BASE: usize = 0xFF84_1000;

/// GIC-400 CPU interface (GICC).
pub const GICC_BASE: usize = 0xFF84_2000;

/// ARM Generic Timer physical timer PPI (architecture-defined).
pub const TIMER_PPI: u32 = 30;

/// PL011 UART0 SPI on the GIC-400.
///
/// BCM2711 maps VideoCore peripheral IRQ 57 (UART) to GIC SPI base 96 →
/// absolute interrupt id **153**. Matches Linux `GIC_SPI 121` (32 + 121 = 153).
pub const UART0_SPI: u32 = 153;

/// End of the RAM the kernel identity-maps and may allocate from.
///
/// How much RAM the board has is board knowledge, not architecture knowledge:
/// `arch` describes how AArch64 translates addresses, `bsp` describes what is
/// at those addresses. Two 1 GiB blocks are mapped, so allocation stops at
/// 2 GiB even on 4/8 GB boards until the memory map is discovered at runtime.
pub const IDENTITY_RAM_END: usize = 0x8000_0000;

/// Granule of the ADR-0012 frame pool (matches 4 KiB page tables).
pub const FRAME_SIZE: usize = 0x1000;

/// Frames in the named user/AS phys pool (512 × 4 KiB = 2 MiB).
///
/// Sized for M5 v1 (one small AS + spare). Raise with layout evidence, not by
/// consuming “whatever RAM is left after the heap”.
pub const FRAME_POOL_FRAMES: usize = 512;

/// Byte length of the frame pool (`FRAME_POOL_FRAMES * FRAME_SIZE`).
pub const FRAME_POOL_BYTES: usize = FRAME_POOL_FRAMES * FRAME_SIZE;

/// User virtual window base (ADR-0014) — 1 GiB VA slot.
///
/// Disjoint from the **fine** kernel map (image, stacks, heap, frame pool,
/// devices). `IDENTITY_RAM_END` is the PA ceiling for identity RAM, not a
/// claim that every byte below it is mapped; this window sits in the unmapped
/// gap above the low layout and below the device windows.
pub const USER_VA_BASE: u64 = 0x0000_0000_4000_0000;

/// User stack size in pages (grows down toward [`USER_VA_BASE`]).
pub const USER_STACK_PAGES: usize = 4;

/// Exclusive top of the user stack VA range (`USER_VA_BASE + pages * 4 KiB`).
pub const USER_STACK_TOP: u64 = USER_VA_BASE + (USER_STACK_PAGES as u64) * (FRAME_SIZE as u64);

/// Device MMIO windows the kernel maps: `(base, length, name)`.
///
/// Mapped Device-nGnRnE and never executable. Kept as narrow as the hardware
/// allows: everything outside these windows faults, which is how a stray
/// pointer into the peripheral range becomes a diagnosable exception instead
/// of an unpredictable side effect on a device register.
pub const DEVICE_REGIONS: [(usize, usize, &str); 2] = [
    // Low peripherals: GPIO, PL011, mailboxes.
    (PERIPHERAL_BASE, 0x0100_0000, "peripherals"),
    // GIC-400 distributor + CPU interface.
    (0xFF84_0000, 0x0000_4000, "GIC"),
];

// The 1 GiB block constants that used to live here are gone: RAM is no longer
// mapped a gigabyte at a time. `mm::layout` derives the RAM regions from the
// linker symbols so each one can carry its own permissions.
