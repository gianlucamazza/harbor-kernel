//! BCM2711 physical memory map (Raspberry Pi 4 Model B).
//!
//! Peripheral window base is `0xFE00_0000` (Pi 4). Pi 2/3 used `0x3F00_0000`;
//! those constants must not be reused here.

/// Start of the low peripheral MMIO window.
pub const PERIPHERAL_BASE: usize = 0xFE00_0000;

/// What each 1 GiB block of the early identity map covers, from PA 0 up.
///
/// This is board knowledge and nothing else: three gigabytes of RAM, then the
/// gigabyte holding the low peripherals (`0xFE00_0000`) and the GIC
/// (`0xFF84_0000`). It used to be written out inside `arch::mmu`, which is the
/// tree reserved for CPU and ISA — finding F23, and the reason `mm::early`
/// exists.
///
/// Three gigabytes rather than the two of [`IDENTITY_RAM_END`] on purpose: the
/// firmware places the device tree wherever it likes, and the early map has to
/// be able to read it. The kernel map that replaces this one covers far less.
pub const EARLY_BLOCKS: [kernel_core::paging::MemKind; 4] = {
    use kernel_core::paging::MemKind::{Device, NormalWb};
    [NormalWb, NormalWb, NormalWb, Device]
};

/// Granule for page tables, frame pool, and agent MMIO maps (4 KiB).
pub const FRAME_SIZE: usize = 0x1000;

/// GPIO controller.
pub const GPIO_BASE: usize = PERIPHERAL_BASE + 0x0020_0000;

/// PL011 UART0.
pub const UART0_BASE: usize = PERIPHERAL_BASE + 0x0020_1000;

/// PL011 register block size for **agent** Stage-1 maps (ADR-0013).
///
/// The hardware block is smaller; one 4 KiB page is the Stage-1 granule. Kernel
/// EL1 may still sit inside the coarse peripherals window — agents must not.
pub const UART0_REG_BYTES: usize = FRAME_SIZE;

/// User VA where an EL0 agent maps the PL011 page (identity PA = [`UART0_BASE`]).
///
/// Disjoint from [`USER_VA_BASE`] stack/text window and from kernel identity RAM.
pub const USER_PL011_VA: u64 = 0x0000_0000_5000_0000;

/// Power-management block: reset cause, watchdog, reboot partition.
///
/// Inside the `peripherals` window above, so it needs no mapping of its own.
/// Read-only from this kernel — see `drivers::pm` for why that is structural
/// rather than a convention.
pub const PM_BASE: usize = PERIPHERAL_BASE + 0x0010_0000;

/// RNG200 (iproc-rng200) register block — 0x28 bytes.
///
/// Low peripheral mode: legacy bus `0x7E10_4000` → ARM `0xFE10_4000`.
/// Covered by the existing peripherals Device window (no extra map for EL1).
pub const RNG200_BASE: usize = PERIPHERAL_BASE + 0x0010_4000;

/// RNG200 register block size for **agent** Stage-1 maps (ADR-0013 / ADR-0034).
pub const RNG200_REG_BYTES: usize = FRAME_SIZE;

/// User VA where an EL0 agent maps the RNG200 page (PA = [`RNG200_BASE`]).
///
/// Disjoint from [`USER_PL011_VA`] and from [`USER_VA_BASE`].
pub const USER_RNG_VA: u64 = 0x0000_0000_5100_0000;

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

/// Total pages of the **default** user window: one of text, three of stack.
///
/// A default rather than the layout: since ADR-0021 an agent declares its own
/// geometry in its manifest entry, and this is what an address space gets when
/// nobody asks. `USER_STACK_TOP` used to sit beside it and no longer does —
/// `UserWindow::stack_top` computes it from a window that is now per agent, and
/// a board constant naming *the* stack top would describe one case out of many.
pub const USER_STACK_PAGES: usize = 4;

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
