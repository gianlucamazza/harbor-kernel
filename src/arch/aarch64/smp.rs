//! K8 SMP: unpark core 1 (ADR-0070) and SGI IPI wake (ADR-0074).
//!
//! First slice: secondary leaves `WFE`, enables the shared kernel map, signals
//! alive. Second slice: after the primary opens the GIC, secondary brings up
//! its banked CPU interface, takes SGI 0, and signals `CORE1_IPI`. Still no
//! per-core runqueue.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::arch::{cache, cpu, exception, mmu};

/// Per-affinity entry addresses for secondaries waiting in `boot.s`.
///
/// Slot 0 is unused (primary). This slice writes only slot 1. Must be
/// `no_mangle` — `boot.s` loads it by symbol name.
#[unsafe(no_mangle)]
static secondary_entry: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// How many times secondaries entered `secondary_wait` (best-effort).
/// Zero under QEMU `-kernel` (secondaries wait in QEMU's own spin table);
/// non-zero on real start4.elf where every core enters our image.
#[unsafe(no_mangle)]
static secondary_seen: AtomicU64 = AtomicU64::new(0);

static CORE1_ALIVE: AtomicBool = AtomicBool::new(false);

/// Primary has finished GICD init + SGI handler registration; secondary may
/// program its banked GICC and unmask IRQs (ADR-0074).
static SECONDARY_MAY_IRQ: AtomicBool = AtomicBool::new(false);

/// Core 1 finished banked GIC bring-up and is ready to take SGI 0.
static SECONDARY_IRQ_READY: AtomicBool = AtomicBool::new(false);

/// Core 1 handled the wake SGI.
static CORE1_IPI: AtomicBool = AtomicBool::new(false);

/// Kernel page-table root PA for the secondary, published + cleaned to PoC by
/// the primary before SEV. Must not use [`mmu::kernel_root_phys`] alone on the
/// secondary while its MMU is still off: that read is Device-nGnRnE and will
/// not snoop the primary's write-back line (HW timeout, `seen=0` if the core
/// only entered via the spin-table and then halted on a zero root).
static SECONDARY_ROOT_PHYS: AtomicU64 = AtomicU64::new(0);

/// Core 1 stack storage. Must live in **BSS** (writable): a zeroed
/// `static` of this size was landing in `.rodata`, and the secondary's first
/// spill faulted before `CORE1_ALIVE` could be set.
#[repr(C, align(16))]
struct Core1Stack([u8; 16 * 1024]);

/// Exception stack (SP_EL1) for core 1 — symbol for `boot.s`.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss.core1_stack")]
static CORE1_EXC_STACK: Core1Stack = Core1Stack([0; 16 * 1024]);

/// Kernel stack (SP_EL0) for core 1 — symbol for `boot.s`.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".bss.core1_stack")]
static CORE1_KER_STACK: Core1Stack = Core1Stack([0; 16 * 1024]);

/// Entry run on core 1 after EL2→EL1 and stacks (from `secondary_el2_entry`).
///
/// # Safety
/// Called only on affinity 1, with IRQs masked, before any other core-1 code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn secondary_main() -> ! {
    // Prefer the handoff word the primary cleaned to PoC; fall back to the
    // mmu ROOT only if somehow already coherent (e.g. both caches off).
    let root = match SECONDARY_ROOT_PHYS.load(Ordering::Acquire) {
        0 => match mmu::kernel_root_phys() {
            Some(r) => r as u64,
            None => cpu::halt(),
        },
        r => r,
    };
    if root == 0 {
        cpu::halt();
    }
    // SAFETY: same tables primary activated; identity-mapped text/data.
    unsafe {
        mmu::enable_existing(root);
    }
    exception::init();
    CORE1_ALIVE.store(true, Ordering::Release);
    // SAFETY: cache maintenance + barrier on our own static — makes the alive
    // flag visible to the primary (Normal WB, both MMUs on — hardware
    // coherency; still clean to PoC for belt-and-braces).
    unsafe {
        cache::clean_dcache_poc(core::ptr::addr_of!(CORE1_ALIVE) as usize, 8);
        core::arch::asm!("dsb ish", options(nostack, preserves_flags));
    }

    // Policy idle (GIC banked bring-up + WFI) lives in the board IRQ bind —
    // arch must not import bsp/drivers (layering). Symbol seam like boot.s.
    unsafe extern "C" {
        fn harbor_secondary_idle() -> !;
    }
    // SAFETY: product image always provides the symbol; IRQs still masked.
    unsafe { harbor_secondary_idle() }
}

/// QEMU `raspi*_64` spin-table base (absolute PA). Secondary *n* polls
/// `SPIN_TABLE + 8*n` (see QEMU `write_smpboot64`). Core 1 → `0xe0`.
const QEMU_SPIN_TABLE: usize = 0xd8;
const QEMU_SPIN_CORE1: usize = QEMU_SPIN_TABLE + 8; // 0xe0

/// BCM2711 ARM local: core1 mailbox-3 write-set (firmware-style poke).
const CORE1_MBOX3_SET: usize = 0xFF80_0000 + 0x9C;

/// Publish core 1's entry point and wait until it signals alive.
///
/// Call only on core 0 after the kernel map is active and `exception::init`
/// has run on the primary. Returns whether core 1 set the alive flag.
///
/// Wake paths (all attempted):
/// 1. In-kernel `secondary_entry[1]` + SEV — cores already in `_start` wait
///    (real start4.elf: all cores enter the image).
/// 2. QEMU AArch64 spin-table word at PA `0xe0` + SEV.
/// 3. ARM-local mailbox 3 write-set (Pi firmware mailbox path).
pub fn unpark_core1() -> bool {
    unsafe extern "C" {
        fn secondary_el2_entry();
    }
    let entry = secondary_el2_entry as *const () as usize as u64;
    let Some(root) = mmu::kernel_root_phys() else {
        return false;
    };
    // Handoff root first — secondary must see this with MMU off.
    SECONDARY_ROOT_PHYS.store(root as u64, Ordering::Release);
    // Path A: secondaries already in `secondary_wait` (real start4.elf).
    secondary_entry[1].store(entry, Ordering::Release);
    // Path B: QEMU / firmware spin-table (PA in low RAM, mapped "low RAM").
    // SAFETY: page 0 is in the fine map as Normal RW for this reason.
    unsafe {
        core::ptr::write_volatile(QEMU_SPIN_CORE1 as *mut u64, entry);
    }
    // Path C: ARM local mailbox (mapped Device window).
    // SAFETY: ARM local region is in DEVICE_REGIONS.
    unsafe {
        core::ptr::write_volatile(CORE1_MBOX3_SET as *mut u32, entry as u32);
    }
    // Publish every cacheable handoff word to PoC before SEV.
    // SAFETY: all Normal-mapped (ROOT handoff + entry table in BSS; spin PA).
    unsafe {
        cache::clean_dcache_poc(core::ptr::addr_of!(SECONDARY_ROOT_PHYS) as usize, 8);
        cache::clean_dcache_poc(core::ptr::addr_of!(secondary_entry[1]) as usize, 8);
        cache::clean_dcache_poc(QEMU_SPIN_CORE1, 8);
        core::arch::asm!("dsb ish", "sev", options(nostack, preserves_flags));
    }

    // HW secondaries can be slower than QEMU; ~few seconds of spinning.
    let budget: u64 = 200_000_000;
    let mut spins = 0u64;
    while spins < budget {
        if CORE1_ALIVE.load(Ordering::Acquire) {
            return true;
        }
        if spins.is_multiple_of(100_000) {
            // SAFETY: event signal only.
            unsafe {
                core::arch::asm!("sev", options(nostack, preserves_flags));
            }
        }
        core::hint::spin_loop();
        spins += 1;
    }
    CORE1_ALIVE.load(Ordering::Acquire)
}

/// Secondaries observed at `secondary_wait` (0 under QEMU `-kernel`;
/// non-zero on real start4.elf where all cores enter the image).
#[inline]
pub fn secondary_seen_count() -> u64 {
    secondary_seen.load(Ordering::Acquire)
}

/// Allow core 1 to program its banked GICC and unmask IRQs (ADR-0074).
pub fn release_secondary_irq_bringup() {
    SECONDARY_MAY_IRQ.store(true, Ordering::Release);
    // SAFETY: PoC-clean + SEV so a secondary parked on WFE sees the flag.
    unsafe {
        cache::clean_dcache_poc(core::ptr::addr_of!(SECONDARY_MAY_IRQ) as usize, 8);
        core::arch::asm!("dsb ish", "sev", options(nostack, preserves_flags));
    }
}

/// Whether the primary has authorised secondary IRQ bring-up.
#[inline]
pub fn secondary_may_irq() -> bool {
    SECONDARY_MAY_IRQ.load(Ordering::Acquire)
}

/// Core 1 finished banked GIC init and is about to unmask.
pub fn mark_secondary_irq_ready() {
    SECONDARY_IRQ_READY.store(true, Ordering::Release);
    // SAFETY: as release_secondary_irq_bringup — primary polls this flag.
    unsafe {
        cache::clean_dcache_poc(core::ptr::addr_of!(SECONDARY_IRQ_READY) as usize, 8);
        core::arch::asm!("dsb ish", options(nostack, preserves_flags));
    }
}

/// Spin until core 1 reports banked GIC ready, or the budget expires.
pub fn wait_secondary_irq_ready(budget: u64) -> bool {
    let mut spins = 0u64;
    while spins < budget {
        if SECONDARY_IRQ_READY.load(Ordering::Acquire) {
            return true;
        }
        if spins.is_multiple_of(100_000) {
            // SAFETY: nudge a secondary still in WFE.
            unsafe {
                core::arch::asm!("sev", options(nostack, preserves_flags));
            }
        }
        core::hint::spin_loop();
        spins += 1;
    }
    SECONDARY_IRQ_READY.load(Ordering::Acquire)
}

/// Handler path: core 1 took the wake SGI.
#[inline]
pub fn note_core1_ipi() {
    CORE1_IPI.store(true, Ordering::Release);
    // SAFETY: primary may poll without snoop if caches are eccentric.
    unsafe {
        cache::clean_dcache_poc(core::ptr::addr_of!(CORE1_IPI) as usize, 8);
        core::arch::asm!("dsb ish", options(nostack, preserves_flags));
    }
}

/// Spin until the IPI flag is set, or the budget expires.
pub fn wait_core1_ipi(budget: u64) -> bool {
    let mut spins = 0u64;
    while spins < budget {
        if CORE1_IPI.load(Ordering::Acquire) {
            return true;
        }
        core::hint::spin_loop();
        spins += 1;
    }
    CORE1_IPI.load(Ordering::Acquire)
}
