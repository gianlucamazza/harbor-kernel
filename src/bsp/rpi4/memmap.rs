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

/// Physical base of each 1 GiB block the kernel identity-maps as RAM.
///
/// Documented for board consumers; the active programmer is `arch::mmu`
/// (must stay free of BSP imports).
#[allow(dead_code)]
pub const IDENTITY_RAM_BLOCKS: [u64; 2] = [0x0000_0000, 0x4000_0000];

/// Physical base of the 1 GiB block holding the peripheral and GIC windows.
///
/// `PERIPHERAL_BASE` (`0xFE00_0000`) and the GIC (`0xFF84_x000`) both fall in
/// the block starting at `0xC000_0000`.
#[allow(dead_code)]
pub const DEVICE_BLOCK: u64 = 0xC000_0000;
