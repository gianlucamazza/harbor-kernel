//! CPU helpers for x86_64 lab (ADR-0071).
//!
//! Full IRQ/idle surface for the arch contract; L0 uses halt, CLI, CPUID.

#![allow(dead_code)] // contract helpers (irq_save, etc.) unused until sched/irq land

use core::arch::asm;

#[inline(always)]
pub fn wait_for_event() {
    // SAFETY: hlt with interrupts masked is a pure idle.
    unsafe {
        asm!("hlt", options(nostack, nomem, preserves_flags));
    }
}

#[inline(always)]
pub fn wait_for_interrupt() {
    wait_for_event();
}

#[inline]
#[must_use]
pub fn irq_save() -> u64 {
    let rflags: u64;
    // SAFETY: read RFLAGS and clear IF.
    unsafe {
        asm!(
            "pushfq",
            "pop {r}",
            "cli",
            r = out(reg) rflags,
            options(nostack, preserves_flags),
        );
    }
    rflags
}

/// # Safety
/// `token` must come from [`irq_save`] on this core.
#[inline]
pub unsafe fn irq_restore(token: u64) {
    // SAFETY: restore IF from captured RFLAGS.
    unsafe {
        if token & (1 << 9) != 0 {
            asm!("sti", options(nostack, preserves_flags));
        } else {
            asm!("cli", options(nostack, preserves_flags));
        }
    }
}

#[inline]
pub fn without_irqs<R>(f: impl FnOnce() -> R) -> R {
    let t = irq_save();
    let r = f();
    // SAFETY: paired with irq_save above.
    unsafe {
        irq_restore(t);
    }
    r
}

#[inline]
pub fn irq_disable() {
    // SAFETY: clear IF.
    unsafe {
        asm!("cli", options(nostack, nomem, preserves_flags));
    }
}

#[inline]
pub fn halt() -> ! {
    loop {
        wait_for_event();
    }
}

#[inline]
pub fn sync_pipeline() {
    // SAFETY: serialising instruction.
    unsafe {
        asm!("mfence", options(nostack, nomem, preserves_flags));
    }
}

/// Raw CPUID leaf (eax in, eax/ebx/ecx/edx out).
#[inline]
pub fn cpuid(leaf: u32) -> (u32, u32, u32, u32) {
    let a: u32;
    let b: u32;
    let c: u32;
    let d: u32;
    // SAFETY: CPUID is non-privileged. Preserve RBX (SysV callee-saved) via push/pop.
    unsafe {
        asm!(
            "push rbx",
            "cpuid",
            "mov {b:e}, ebx",
            "pop rbx",
            inout("eax") leaf => a,
            b = out(reg) b,
            lateout("ecx") c,
            lateout("edx") d,
            options(nostack, preserves_flags),
        );
    }
    (a, b, c, d)
}

/// Vendor string from CPUID leaf 0 (12 ASCII bytes).
pub fn vendor_id() -> [u8; 12] {
    let (_a, b, c, d) = cpuid(0);
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&b.to_le_bytes());
    out[4..8].copy_from_slice(&d.to_le_bytes());
    out[8..12].copy_from_slice(&c.to_le_bytes());
    out
}

/// CPUID leaf 1 eax (family/model/stepping encoding).
#[inline]
pub fn cpuid_leaf1_eax() -> u32 {
    cpuid(1).0
}
