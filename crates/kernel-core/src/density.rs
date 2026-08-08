//! Agent density arithmetic (ADR-0044 / K5) — pure, host-tested.
//!
//! Stack classes trade usable kernel stack for more concurrent tasks.

use crate::paging::PAGE_SIZE;

/// Kernel stack class chosen at spawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackClass {
    /// 16 KiB usable — default full agent/driver depth.
    Full,
    /// 4 KiB usable — short workers / shallow agents.
    Thin,
}

/// Usable stack bytes (not including the unmapped guard page).
pub const fn usable_bytes(class: StackClass) -> usize {
    match class {
        StackClass::Full => 16 * 1024,
        StackClass::Thin => 4 * 1024,
    }
}

/// Total heap bytes consumed per task (usable + one guard page).
pub const fn bytes_per_task(class: StackClass) -> usize {
    usable_bytes(class) + PAGE_SIZE as usize
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
    fn thin_fits_more_than_full() {
        let heap = 256 * 1024;
        let full = max_tasks_for_heap(heap, StackClass::Full);
        let thin = max_tasks_for_heap(heap, StackClass::Thin);
        assert!(thin > full);
        assert_eq!(thin_advantage(heap), thin - full);
    }

    #[test]
    fn empty_heap_fits_none() {
        assert_eq!(max_tasks_for_heap(0, StackClass::Thin), 0);
    }
}
