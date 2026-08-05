//! Cache maintenance required around enabling translation.
//!
//! Before `SCTLR_EL1.{M,C,I}` are set, the kernel has been running with caches
//! off: its writes went straight to memory, but the lines the platform
//! firmware left behind are still resident. Turning the caches on without
//! invalidating them first lets a stale line shadow real memory — including
//! the page table the walker is about to read.

use core::arch::asm;

/// Invalidate the entire instruction cache and the branch predictor.
///
/// # Safety
/// Discards cached instructions; correct only at boot, before enabling the
/// MMU, or after writing executable memory.
pub unsafe fn invalidate_icache() {
    unsafe {
        asm!(
            "ic iallu",
            "dsb ish",
            "isb",
            options(nostack, preserves_flags)
        );
    }
}

/// Invalidate every data cache level to the point of coherency, by set/way.
///
/// Invalidate rather than clean: with caches off the kernel's own writes never
/// entered a cache, so there is nothing of ours to write back, and anything
/// resident is firmware-era state we must not let shadow memory.
///
/// # Safety
/// Discards cached data without writing it back. Valid only at boot, before
/// the MMU and the data cache are enabled.
pub unsafe fn invalidate_dcache_all() {
    unsafe {
        let clidr: u64;
        asm!("mrs {}, clidr_el1", out(reg) clidr, options(nomem, nostack));

        // Level of Coherency: the outermost level that must be maintained.
        let loc = (clidr >> 24) & 0b111;

        for level in 0..loc {
            // Cache type for this level: 2 = data, 3 = separate I/D, 4 = unified.
            let ctype = (clidr >> (3 * level)) & 0b111;
            if ctype < 2 {
                continue;
            }

            // Select the level, then read its geometry.
            let csselr = level << 1;
            asm!("msr csselr_el1, {}", "isb", in(reg) csselr, options(nostack));

            let ccsidr: u64;
            asm!("mrs {}, ccsidr_el1", out(reg) ccsidr, options(nomem, nostack));

            // LineSize is log2(line bytes) - 4; the set index starts above it.
            let line_shift = (ccsidr & 0b111) + 4;
            let max_way = (ccsidr >> 3) & 0x3FF;
            let max_set = (ccsidr >> 13) & 0x7FFF;

            // The way index sits in the top bits of the set/way operand.
            let way_shift = (max_way as u32).leading_zeros() as u64;

            for way in 0..=max_way {
                for set in 0..=max_set {
                    let sw = (way << way_shift) | (set << line_shift) | (level << 1);
                    asm!("dc isw, {}", in(reg) sw, options(nostack, preserves_flags));
                }
            }
        }

        asm!("dsb sy", "isb", options(nostack, preserves_flags));
    }
}

/// Invalidate the entire EL1 TLB.
///
/// # Safety
/// Only meaningful with a valid translation regime installed or about to be.
pub unsafe fn invalidate_tlb_all() {
    unsafe {
        asm!(
            "dsb ishst",
            "tlbi vmalle1",
            "dsb ish",
            "isb",
            options(nostack, preserves_flags)
        );
    }
}

/// Minimum data-cache line size in bytes from `CTR_EL0.DminLine` (log2 words).
#[inline]
fn dcache_line_size() -> usize {
    let ctr: u64;
    // SAFETY: system register read.
    unsafe {
        asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack, preserves_flags));
    }
    4usize << (ctr as usize & 0xf)
}

/// Clean data cache by VA to the point of unification for `[va, va + len)`.
///
/// Required after the kernel writes memory that will be fetched as instructions
/// (I-cache is not coherent with D-cache on Cortex-A72).
///
/// # Safety
/// `va..va+len` must be mapped Normal memory; translation and D-cache on.
pub unsafe fn clean_dcache_pou(va: usize, len: usize) {
    if len == 0 {
        return;
    }
    let line = dcache_line_size();
    let start = va & !(line - 1);
    let end = va.saturating_add(len);
    let mut p = start;
    unsafe {
        while p < end {
            asm!("dc cvau, {}", in(reg) p, options(nostack, preserves_flags));
            p += line;
        }
        asm!("dsb ish", options(nostack, preserves_flags));
    }
}

/// Make kernel stores to `[va, va + len)` visible to instruction fetch.
///
/// Clean D to PoU, invalidate I (full), order with `isb`. Used after writing
/// EL0 text through the identity map.
///
/// # Safety
/// Same as [`clean_dcache_pou`]; may discard unrelated I-cache lines.
pub unsafe fn publish_executable(va: usize, len: usize) {
    unsafe {
        clean_dcache_pou(va, len);
        invalidate_icache();
    }
}
