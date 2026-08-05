//! ARM GICv2 (GIC-400) — [`IrqChip`] implementation.
//!
//! On BCM2711 bare metal (NS EL1), the path that matches observed hardware is:
//! - PPIs in **Group 0** (IGROUPR bit clear)
//! - claim/EOI via **IAR / EOIR** (not AIAR/AEOIR)
//!
//! Evidence: HPPIR reports PPI 30 while timer pending, but AIAR/IAR-as-G1 claim
//! did not advance ticks. Group 0 + IAR is the classic bare-metal sequence used
//! by working RPi4 examples.

// Audit debt (2026-08-06): 3 unsafe blocks here predate
// `clippy::undocumented_unsafe_blocks` and do not yet say what makes them sound.
// This comes off when the audit reaches this module and the SAFETY comments can
// state something checkable rather than restate the code. See Cargo.toml.
#![allow(clippy::undocumented_unsafe_blocks)]

use kernel_core::gic;

use crate::arch::mmio::Mmio;
use crate::irq::{Ack, IrqChip};

const GICD_CTLR: usize = 0x000;
const GICD_IGROUPR: usize = 0x080;
const GICD_ISENABLER: usize = 0x100;
const GICD_ICENABLER: usize = 0x180;
const GICD_ICPENDR: usize = 0x280;
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ITARGETSR: usize = 0x800;
const GICD_ICFGR: usize = 0xC00;

const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_BPR: usize = 0x008;
const GICC_IAR: usize = 0x00C;
const GICC_EOIR: usize = 0x010;
#[cfg(feature = "bringup")]
const GICC_HPPIR: usize = 0x018;

const CTLR_ENABLE_GRP0: u32 = 1 << 0;
const CTLR_ENABLE_GRP1: u32 = 1 << 1;

pub struct GicV2 {
    dist: Mmio,
    cpu: Mmio,
}

impl GicV2 {
    /// # Safety
    /// `dist_base` / `cpu_base` must be GICD/GICC MMIO for the lifetime of use.
    pub const unsafe fn new(dist_base: usize, cpu_base: usize) -> Self {
        Self {
            dist: unsafe { Mmio::new(dist_base) },
            cpu: unsafe { Mmio::new(cpu_base) },
        }
    }
}

impl IrqChip for GicV2 {
    fn init(&self) {
        // Banked SGI+PPI → Group 0 (IAR/EOIR path).
        self.dist.write32(GICD_IGROUPR, 0x0000_0000);

        self.cpu.write32(GICC_PMR, 0xFF);
        self.cpu.write32(GICC_BPR, 0);

        // Force classic EOI, enable groups (overwrite EOImode from firmware).
        self.cpu
            .write32(GICC_CTLR, CTLR_ENABLE_GRP0 | CTLR_ENABLE_GRP1);
        self.dist
            .write32(GICD_CTLR, CTLR_ENABLE_GRP0 | CTLR_ENABLE_GRP1);
    }

    fn enable(&self, irq: u32) {
        self.set_group0(irq);
        // Highest priority (0) so PMR never filters it.
        self.set_priority(irq, 0x00);
        // SPIs must target a CPU; PPIs and SGIs are banked per core.
        if gic::classify(irq) == gic::IrqClass::Spi {
            self.set_target_cpu0(irq);
            self.set_level_sensitive(irq);
        }
        // Mask before re-enabling: the line may already be live with the old
        // configuration, and clearing a stale pending bit under it is racy.
        self.disable(irq);
        self.clear_pending(irq);

        let (offset, mask) = gic::bit_slot(irq);
        self.dist.write32(GICD_ISENABLER + offset, mask);
    }

    fn disable(&self, irq: u32) {
        let (offset, mask) = gic::bit_slot(irq);
        self.dist.write32(GICD_ICENABLER + offset, mask);
    }

    fn claim(&self) -> Option<Ack> {
        let raw = self.cpu.read32(GICC_IAR);
        // `gic::is_spurious` rather than an inline mask: it knows about both
        // spurious encodings (1023 and 1022, the other security group), and it
        // is the version the host tests actually exercise. Re-deriving the
        // mask here would leave those tests covering code nobody runs.
        if gic::is_spurious(raw) {
            None
        } else {
            Some(Ack(raw))
        }
    }

    fn end(&self, ack: Ack) {
        self.cpu.write32(GICC_EOIR, ack.raw());
        unsafe {
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        }
    }
}

/// Diagnostic accessors, compiled only with the `bringup` feature.
///
/// These read and write GIC registers with side effects (`IAR` claims, `EOIR`
/// completes) and have no place in the driver's stable surface: a caller that
/// reaches them from kernel policy has bypassed the irqchip abstraction.
#[cfg(feature = "bringup")]
impl GicV2 {
    /// Highest pending id without claiming it.
    pub fn debug_hppir_id(&self) -> Option<u32> {
        let raw = self.cpu.read32(GICC_HPPIR);
        if gic::is_spurious(raw) {
            None
        } else {
            Some(gic::ack_id(raw))
        }
    }

    /// Raw IAR (claims). Bring-up / selftest only.
    pub fn debug_iar(&self) -> u32 {
        self.cpu.read32(GICC_IAR)
    }

    /// Raw EOIR. Bring-up / selftest only.
    pub fn debug_eoir(&self, val: u32) {
        self.cpu.write32(GICC_EOIR, val);
        unsafe {
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        }
    }
}

/// Distributor register plumbing used by [`IrqChip::enable`].
impl GicV2 {
    fn set_group0(&self, irq: u32) {
        let (offset, mask) = gic::bit_slot(irq);
        let value = self.dist.read32(GICD_IGROUPR + offset) & !mask;
        self.dist.write32(GICD_IGROUPR + offset, value);
    }

    fn set_priority(&self, irq: u32, priority: u8) {
        let (offset, _) = gic::byte_slot(irq);
        let value = self.dist.read32(GICD_IPRIORITYR + offset);
        self.dist.write32(
            GICD_IPRIORITYR + offset,
            gic::insert_byte(value, irq, priority),
        );
    }

    fn clear_pending(&self, irq: u32) {
        let (offset, mask) = gic::bit_slot(irq);
        self.dist.write32(GICD_ICPENDR + offset, mask);
    }

    /// Route SPI `irq` to CPU interface 0 (bit 0 of the target byte).
    fn set_target_cpu0(&self, irq: u32) {
        let (offset, _) = gic::byte_slot(irq);
        let value = self.dist.read32(GICD_ITARGETSR + offset);
        self.dist
            .write32(GICD_ITARGETSR + offset, gic::insert_byte(value, irq, 0x01));
    }

    /// Level-sensitive configuration (PL011 and most peripherals).
    ///
    /// ICFGR: 2 bits per IRQ; `0b00` = level, `0b10` = edge.
    fn set_level_sensitive(&self, irq: u32) {
        let word = (irq / 16) as usize;
        let shift = (irq % 16) * 2;
        let offset = GICD_ICFGR + word * 4;
        let mut value = self.dist.read32(offset);
        value &= !(0b11 << shift);
        self.dist.write32(offset, value);
    }
}
