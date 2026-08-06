//! Cache maintenance required around enabling translation.
//!
//! Before `SCTLR_EL1.{M,C,I}` are set, the kernel has been running with caches
//! off: its writes went straight to memory, but the lines the platform
//! firmware left behind are still resident. Turning the caches on without
//! invalidating them first lets a stale line shadow real memory — including
//! the page table the walker is about to read.

//! ## Shareability
//!
//! Every maintenance operation here is broadcast (`is`) and paired with an
//! inner-shareable barrier. Three of them used to be the local variants
//! (`tlbi vmalle1`, `ic iallu`) closed by `dsb ish`, which orders a domain the
//! operation never reached. On one core the two are indistinguishable, so
//! nothing chose between them — the local forms were simply what got typed, and
//! `mmu::publish_and_invalidate` had already picked `vaae1is` for its per-page
//! path. Matching them costs nothing with the secondary cores parked in `wfe`
//! and stops being free of consequence the day one of them starts.

use core::arch::asm;

/// Invalidate the entire instruction cache and the branch predictor.
///
/// # Safety
/// Discards cached instructions; correct only at boot, before enabling the
/// MMU, or after writing executable memory.
pub unsafe fn invalidate_icache() {
    // SAFETY: discarding the I-cache is always architecturally legal — the
    // hazard is a stale *fetch*, which the caller's obligation covers. `dsb ish`
    // then `isb` is what makes the invalidation apply to instructions fetched
    // after this returns, rather than to some point later in the pipeline.
    unsafe {
        asm!(
            "ic ialluis",
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
    // SAFETY: every register read here (`clidr_el1`, `ccsidr_el1`) is legal at
    // EL1, and `csselr_el1` only selects which cache level `ccsidr_el1` then
    // describes — it changes no state the rest of the kernel observes. The
    // destructive part is `dc isw`, whose obligation (nothing of ours is dirty,
    // because the caches have been off) is the caller's, stated above. The loop
    // bounds come from the geometry the hardware just reported, so no set/way
    // operand names a line that does not exist.
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
    // SAFETY: discarding TLB entries can never make a translation wrong, only
    // slower — the entries are a cache of the tables, and the tables are the
    // truth. `dsb ishst` first so that table writes already issued are visible
    // to the walker before the invalidation, `dsb ish` + `isb` after so the
    // next fetch cannot use an entry from before it.
    unsafe {
        asm!(
            "dsb ishst",
            "tlbi vmalle1is",
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
    // SAFETY: `dc cvau` cleans a line to the point of unification and cannot
    // lose data — a clean writes back, it does not discard. The caller
    // guarantees the range is mapped Normal memory, which is what makes the
    // operation defined rather than a fault. `p` walks from a line-aligned
    // start in line-sized steps and stops at `end`, so every operand lies in
    // the requested range.
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
    // SAFETY: both halves forward the caller's obligation. The order is the
    // load-bearing part: clean D to the point of unification *before*
    // invalidating I, or the fetch can refill from memory that the store has
    // not reached yet. On Cortex-A72 the two caches are not coherent, so this
    // sequence is what makes a freshly written instruction executable at all.
    unsafe {
        clean_dcache_pou(va, len);
        invalidate_icache();
    }
}
