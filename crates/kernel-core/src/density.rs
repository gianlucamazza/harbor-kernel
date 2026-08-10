//! Agent density arithmetic (ADR-0044 / K5-S ADR-0085/0086) — pure, host-tested.
//!
//! Stack classes trade usable kernel stack for more concurrent tasks. Raising
//! a task-table constant is not a density win (ADR-0085 §2).

use crate::paging::PAGE_SIZE;

/// Kernel stack class chosen at spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackClass {
    /// 16 KiB usable — default full agent/driver depth.
    Full,
    /// 4 KiB usable — short workers / shallow agents (ADR-0044).
    Thin,
    /// One mapped page, **no** unmapped guard (ADR-0085 **K5-S** / ADR-0086).
    ///
    /// On a 4 KiB granule, sub-page “2 KiB + guard” cannot unmap half a page;
    /// Mini therefore trades the guard hole for half of Thin’s heap cost.
    /// Short EL1 workers only — not deep multi-SVC driver loops.
    Mini,
}

/// Usable stack bytes (mapped RW region the SP grows down through).
pub const fn usable_bytes(class: StackClass) -> usize {
    match class {
        StackClass::Full => 16 * 1024,
        StackClass::Thin | StackClass::Mini => 4 * 1024,
    }
}

/// Whether this class carves an unmapped guard page below the usable region.
#[inline]
pub const fn has_guard_page(class: StackClass) -> bool {
    !matches!(class, StackClass::Mini)
}

/// Total heap bytes consumed per task (usable + optional guard page).
pub const fn bytes_per_task(class: StackClass) -> usize {
    let usable = usable_bytes(class);
    if has_guard_page(class) {
        usable + PAGE_SIZE as usize
    } else {
        usable
    }
}

/// How many tasks of `class` fit in `heap_bytes` (integer division).
pub const fn max_tasks_for_heap(heap_bytes: usize, class: StackClass) -> usize {
    let each = bytes_per_task(class);
    match heap_bytes.checked_div(each) {
        Some(n) => n,
        None => 0,
    }
}

/// Extra tasks of `Thin` vs `Full` for the same heap budget.
pub const fn thin_advantage(heap_bytes: usize) -> usize {
    max_tasks_for_heap(heap_bytes, StackClass::Thin)
        .saturating_sub(max_tasks_for_heap(heap_bytes, StackClass::Full))
}

/// Extra tasks of `Mini` vs `Thin` for the same heap budget.
pub const fn mini_advantage(heap_bytes: usize) -> usize {
    max_tasks_for_heap(heap_bytes, StackClass::Mini)
        .saturating_sub(max_tasks_for_heap(heap_bytes, StackClass::Thin))
}

/// Whether `class` is the short-worker band (Thin or Mini), not Full drivers.
#[inline]
pub const fn is_short_worker(class: StackClass) -> bool {
    matches!(class, StackClass::Thin | StackClass::Mini)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_is_16k_plus_guard() {
        assert_eq!(usable_bytes(StackClass::Full), 16 * 1024);
        assert_eq!(bytes_per_task(StackClass::Full), 16 * 1024 + 4096);
    }

    #[test]
    fn thin_is_4k_plus_guard() {
        assert_eq!(usable_bytes(StackClass::Thin), 4 * 1024);
        assert_eq!(bytes_per_task(StackClass::Thin), 4 * 1024 + 4096);
    }

    #[test]
    fn mini_is_one_page_no_guard() {
        assert_eq!(usable_bytes(StackClass::Mini), 4 * 1024);
        assert_eq!(bytes_per_task(StackClass::Mini), 4 * 1024);
        assert!(!has_guard_page(StackClass::Mini));
        assert!(has_guard_page(StackClass::Thin));
    }

    #[test]
    fn mini_fits_more_than_thin_fits_more_than_full() {
        let heap = 256 * 1024;
        let full = max_tasks_for_heap(heap, StackClass::Full);
        let thin = max_tasks_for_heap(heap, StackClass::Thin);
        let mini = max_tasks_for_heap(heap, StackClass::Mini);
        assert!(thin > full);
        assert!(mini > thin);
        assert_eq!(thin_advantage(heap), thin - full);
        assert_eq!(mini_advantage(heap), mini - thin);
        // Mini is half Thin's heap per task (4 KiB vs 8 KiB).
        assert_eq!(
            bytes_per_task(StackClass::Thin),
            2 * bytes_per_task(StackClass::Mini)
        );
    }

    #[test]
    fn short_worker_band() {
        assert!(!is_short_worker(StackClass::Full));
        assert!(is_short_worker(StackClass::Thin));
        assert!(is_short_worker(StackClass::Mini));
    }

    #[test]
    fn empty_heap_fits_none() {
        assert_eq!(max_tasks_for_heap(0, StackClass::Thin), 0);
        assert_eq!(max_tasks_for_heap(0, StackClass::Mini), 0);
    }
}
