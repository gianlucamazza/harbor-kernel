//! K8 first slice — unpark core 1 into an idle loop ([ADR-0070]).
//!
//! No per-core runqueue, no IPI scheduler: prove the secondary leaves `WFE`,
//! enables the shared kernel map, and signals alive.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::arch::{cpu, exception, mmu};

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

/// How many times secondaries entered `.L_secondary_wait` (best-effort).
/// Non-zero under QEMU means cores past CPU0 are executing our image.
#[unsafe(no_mangle)]
static secondary_seen: AtomicU64 = AtomicU64::new(0);

static CORE1_ALIVE: AtomicBool = AtomicBool::new(false);

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
    // SAFETY: primary published a non-zero root before writing secondary_entry[1].
    let root = match mmu::kernel_root_phys() {
        Some(r) => r as u64,
        None => cpu::halt(),
    };
    // SAFETY: same tables primary activated; identity-mapped text/data.
    unsafe {
        mmu::enable_existing(root);
    }
    exception::init();
    CORE1_ALIVE.store(true, Ordering::Release);
    // IRQs stay masked; WFE until SEV (none expected) — pure park.
    loop {
        cpu::wait_for_event();
    }
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
    // Path A: secondaries already in `.L_secondary_wait`.
    secondary_entry[1].store(entry, Ordering::Release);
    // SAFETY: publish then wake WFE waiters.
    unsafe {
        core::arch::asm!("dsb ish", "sev", options(nostack, preserves_flags));
    }
    // Path B: QEMU spin-table (PA identity-mapped as RAM).
    // SAFETY: low DRAM under the kernel identity map; ROM/zeroed at reset.
    unsafe {
        core::ptr::write_volatile(QEMU_SPIN_CORE1 as *mut u64, entry);
        core::arch::asm!("dsb sy", "sev", options(nostack, preserves_flags));
    }
    // Path C: ARM local mailbox (mapped Device window).
    // SAFETY: ARM local region is in DEVICE_REGIONS.
    unsafe {
        core::ptr::write_volatile(CORE1_MBOX3_SET as *mut u32, entry as u32);
        core::arch::asm!("dsb sy", "sev", options(nostack, preserves_flags));
    }

    let budget: u64 = 50_000_000;
    let mut spins = 0u64;
    while spins < budget {
        if CORE1_ALIVE.load(Ordering::Acquire) {
            return true;
        }
        if spins % 100_000 == 0 {
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

/// Secondaries observed at `.L_secondary_wait` (0 under QEMU `-kernel`;
/// non-zero on real start4.elf where all cores enter the image).
#[inline]
pub fn secondary_seen_count() -> u64 {
    secondary_seen.load(Ordering::Acquire)
}
