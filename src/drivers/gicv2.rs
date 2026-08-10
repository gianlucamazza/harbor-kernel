//! ARM GICv2 (GIC-400) — [`IrqChip`] implementation.
//!
//! On BCM2711 bare metal (NS EL1), the path that matches observed hardware is:
//! - PPIs in **Group 0** (IGROUPR bit clear)
//! - claim/EOI via **IAR / EOIR** (not AIAR/AEOIR)
//!
//! Evidence: HPPIR reports PPI 30 while timer pending, but AIAR/IAR-as-G1 claim
//! did not advance ticks. Group 0 + IAR is the classic bare-metal sequence used
//! by working RPi4 examples.

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
/// Software Generated Interrupt Register (write-only). ADR-0074.
const GICD_SGIR: usize = 0xF00;

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
            // SAFETY: the address is the caller's obligation, forwarded
            // verbatim to `Mmio::new`, which carries the same one. This driver
            // never invents a base — the BSP names both, from the compiled-in
            // board constants (ADR-0011).
            dist: unsafe { Mmio::new(dist_base) },
            // SAFETY: as above, for the CPU interface.
            cpu: unsafe { Mmio::new(cpu_base) },
        }
    }
}

impl IrqChip for GicV2 {
    fn init(&self) {
        // Primary: banked SGI+PPI + this CPU's interface, then open the
        // distributor for the whole chip (shared). Secondaries use
        // [`init_this_cpu`] only — they must not rewrite GICD_CTLR.
        self.init_this_cpu();
        self.dist
            .write32(GICD_CTLR, CTLR_ENABLE_GRP0 | CTLR_ENABLE_GRP1);
    }

    fn enable(&self, irq: u32) {
        // Mask *first*. The comment here used to say exactly that while the
        // code did it fourth: group, priority, target and trigger were all
        // reprogrammed before the line was masked. That is the case the comment
        // is about — `enable_gic=1` means the firmware has already programmed
        // this distributor (ADR-0004), so a line can arrive live with a
        // configuration of someone else's choosing, and changing its trigger
        // mode underneath is precisely what should not happen while it can
        // fire.
        self.disable(irq);

        self.set_group0(irq);
        // Highest priority (0) so PMR never filters it.
        self.set_priority(irq, 0x00);
        // SPIs must target a CPU; PPIs and SGIs are banked per core.
        if gic::classify(irq) == gic::IrqClass::Spi {
            self.set_target_cpu0(irq);
            self.set_level_sensitive(irq);
        }

        // Only now is a stale pending bit safe to drop: nothing can set it
        // again between here and the unmask below.
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
        // SAFETY: a barrier instruction. It is here because the Device-nGnRnE
        // mapping orders this write against other device accesses but not
        // against the normal-memory bookkeeping the handler does next — see the
        // note in `arch::mmio` on why ordering belongs at the call site. Without
        // it the interrupt can be retired after the counters that record it.
        unsafe {
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        }
    }

    fn send_sgi(&self, sgi_id: u32, cpu_target_list: u8) -> bool {
        self.send_sgi_raw(sgi_id, cpu_target_list)
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
        // SAFETY: as [`IrqChip::end`] — a barrier, ordering this completion
        // against what the bring-up probe does afterwards.
        unsafe {
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        }
    }
}

impl GicV2 {
    /// Program **this CPU's** banked GICC + SGI/PPI Group 0 (ADR-0074).
    ///
    /// Call on every core that will take IRQs. Does not touch the shared
    /// distributor enable — primary [`IrqChip::init`] owns that.
    pub fn init_this_cpu(&self) {
        // Banked SGI+PPI → Group 0 (IAR/EOIR path).
        self.dist.write32(GICD_IGROUPR, 0x0000_0000);

        self.cpu.write32(GICC_PMR, 0xFF);
        self.cpu.write32(GICC_BPR, 0);

        // Force classic EOI, enable groups (overwrite EOImode from firmware).
        self.cpu
            .write32(GICC_CTLR, CTLR_ENABLE_GRP0 | CTLR_ENABLE_GRP1);
    }

    /// Raise an SGI on the CPUs named by `cpu_target_list` (one bit per
    /// interface). Encoding is pure in `kernel_core::gic` (ADR-0074).
    ///
    /// Returns `false` if `sgi_id` is not an SGI. Does not require the
    /// sender to have the line enabled; the **target** must.
    pub fn send_sgi_raw(&self, sgi_id: u32, cpu_target_list: u8) -> bool {
        let Some(word) =
            gic::sgir_word(sgi_id, cpu_target_list, gic::SgiFilter::TargetList)
        else {
            return false;
        };
        // Order prior Normal-memory handoff against the device write that
        // may wake another core into that memory.
        // SAFETY: barrier only.
        unsafe {
            core::arch::asm!("dsb ishst", options(nostack, preserves_flags));
        }
        self.dist.write32(GICD_SGIR, word);
        // SAFETY: complete the distributor write before the sender polls.
        unsafe {
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        }
        true
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
