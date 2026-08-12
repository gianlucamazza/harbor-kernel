//! What the loader will do with a manifest, as data (ADR-0097) — pure, host-tested.
//!
//! [`bind`](crate::manifest::bind) answers *may this loader grant this entry*
//! and [`AgentEntry::validate`](crate::manifest::AgentEntry::validate) answers
//! *is this entry self-consistent*. Both are pure already. What was not is the
//! **composition** of them — the order they are asked in, what an empty table
//! means, and which refusal a caller reports — and composition is where a
//! loader gets authority wrong, one call before an agent is handed what it
//! asked for.
//!
//! So the loader no longer decides while it acts. It builds a plan, then walks
//! it: `src/bootstrap/loader.rs` keeps the store parsing, the spawning and the
//! printing, and none of the judgement.
//!
//! # The order is the decision
//!
//! `validate` runs **before** `bind`. An entry that is both malformed and
//! over-reaching is reported as malformed, and the geometry is refused before
//! anything takes frames from the pool for a program that cannot fit in them.
//! That was true before this module existed; the difference is that it is now
//! a test rather than a reading of the loop.

use crate::cap::CapId;
use crate::manifest::{AgentEntry, BindError, MAX_SLOTS, bind};

/// Which manifest a plan was made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// An agent store parsed out of the image (ADR-0029).
    Store { agents: usize },
    /// The table compiled into the kernel.
    Builtin,
}

/// Why an entry will not be spawned.
///
/// One variant per question asked, in the order they are asked: a refusal here
/// says *which* check refused, and the ABI detail the kernel prints comes
/// straight from the inner error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `validate` refused: geometry, image size, or `home_cpu` out of range.
    Invalid(BindError),
    /// `bind` refused: the entry named a capability the loader does not hold.
    Unheld(BindError),
}

/// What the loader will do with one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryPlan {
    /// Spawn it, on `home_cpu` (ADR-0088), with these slots already resolved.
    Spawn {
        /// Index into the active manifest — what the loader remembers per task.
        index: u8,
        home_cpu: u8,
        slots: [Option<CapId>; MAX_SLOTS],
    },
    /// Refuse it, and say which check refused.
    Refuse(Refusal),
}

/// Why no plan could be made at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// The manifest has no entries. A different outcome from a manifest whose
    /// entries all refuse: nothing was asked, so nothing was denied.
    Empty,
    /// `out` is shorter than the table. The caller sizes the buffer from
    /// `MAX_AGENTS`; a short one would silently plan a prefix.
    OutTooSmall { entries: usize, capacity: usize },
}

/// Plan every entry of `table`, writing one [`EntryPlan`] per entry into `out`.
///
/// Returns how many were written. `held` is the loader's vocabulary
/// ([`crate::held`], ADR-0099): the whole of what it may grant, one position per
/// declared authority. An entry naming anything outside it is refused by
/// arithmetic rather than by a check (ADR-0021), and one naming a position that
/// exists and is empty is refused as `HeldVacant` — same `Refusal::Unheld`,
/// different fact inside it.
pub fn plan(
    table: &[AgentEntry],
    held: &[Option<CapId>],
    frame_size: usize,
    out: &mut [EntryPlan],
) -> Result<usize, PlanError> {
    if table.is_empty() {
        return Err(PlanError::Empty);
    }
    if out.len() < table.len() {
        return Err(PlanError::OutTooSmall {
            entries: table.len(),
            capacity: out.len(),
        });
    }

    for (index, entry) in table.iter().enumerate() {
        out[index] = match entry.validate(frame_size) {
            Err(e) => EntryPlan::Refuse(Refusal::Invalid(e)),
            Ok(()) => match bind(entry, held) {
                Err(e) => EntryPlan::Refuse(Refusal::Unheld(e)),
                Ok(slots) => EntryPlan::Spawn {
                    index: index as u8,
                    home_cpu: entry.home_cpu,
                    slots,
                },
            },
        };
    }
    Ok(table.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: usize = 4096;
    static IMAGE: [u8; 8] = [0; 8];

    fn entry(name: &'static str, slots: [Option<u8>; MAX_SLOTS]) -> AgentEntry {
        AgentEntry {
            name,
            image: &IMAGE,
            text_pages: 1,
            stack_pages: 3,
            slots,
            device: None,
            home_cpu: 0,
        }
    }

    /// A vocabulary of two provided positions (ADR-0099 shape).
    fn caps(n: usize) -> [Option<CapId>; 2] {
        let _ = n;
        [Some(CapId::new(1, 1)), Some(CapId::new(2, 1))]
    }

    #[test]
    fn an_empty_manifest_is_its_own_outcome() {
        // Not "zero plans": nothing was asked, so nothing was denied, and the
        // loader prints a different line for it.
        let mut out = [EntryPlan::Refuse(Refusal::Invalid(BindError::BadGeometry {
            text_pages: 0,
            stack_pages: 0,
        })); 4];
        assert_eq!(plan(&[], &caps(2), PAGE, &mut out), Err(PlanError::Empty));
    }

    #[test]
    fn a_short_output_buffer_is_refused_rather_than_truncated() {
        let table = [entry("a", [None; MAX_SLOTS]), entry("b", [None; MAX_SLOTS])];
        let mut out = [EntryPlan::Refuse(Refusal::Invalid(BindError::BadGeometry {
            text_pages: 0,
            stack_pages: 0,
        })); 1];
        assert_eq!(
            plan(&table, &caps(2), PAGE, &mut out),
            Err(PlanError::OutTooSmall {
                entries: 2,
                capacity: 1
            })
        );
    }

    #[test]
    fn an_output_buffer_exactly_the_size_of_the_table_is_enough() {
        // The boundary the caller actually sits on: `loader::load_all` sizes
        // its buffer from `MAX_AGENTS` and the store cannot hold more, so
        // equal-length is the normal case and must not be refused. `<` rather
        // than `<=` is what says so; mutation found this untested, because
        // every other test here passes a buffer with room to spare.
        let table = [entry("a", [None; MAX_SLOTS]), entry("b", [None; MAX_SLOTS])];
        let mut out = [EntryPlan::Refuse(Refusal::Invalid(BindError::BadGeometry {
            text_pages: 0,
            stack_pages: 0,
        })); 2];
        assert_eq!(plan(&table, &caps(2), PAGE, &mut out), Ok(2));
        assert!(matches!(out[0], EntryPlan::Spawn { index: 0, .. }));
        assert!(matches!(out[1], EntryPlan::Spawn { index: 1, .. }));
    }

    #[test]
    fn a_well_formed_entry_is_planned_with_its_slots_and_its_home() {
        let mut e = entry("beacon", [Some(1), None, None, None]);
        e.home_cpu = 1;
        let held = caps(2);
        let mut out = [EntryPlan::Refuse(Refusal::Invalid(BindError::BadGeometry {
            text_pages: 0,
            stack_pages: 0,
        })); 4];

        assert_eq!(plan(&[e], &held, PAGE, &mut out), Ok(1));
        match out[0] {
            EntryPlan::Spawn {
                index,
                home_cpu,
                slots,
            } => {
                assert_eq!(index, 0);
                assert_eq!(home_cpu, 1, "ADR-0088: the entry's home is carried");
                assert_eq!(slots[0], held[1], "slot 0 names held[1]");
                assert_eq!(slots[1], None);
            }
            other => panic!("expected a spawn plan, got {other:?}"),
        }
    }

    #[test]
    fn an_entry_naming_a_capability_the_loader_lacks_is_refused_by_bind() {
        let e = entry("greedy", [Some(9), None, None, None]);
        let mut out = [EntryPlan::Refuse(Refusal::Invalid(BindError::BadGeometry {
            text_pages: 0,
            stack_pages: 0,
        })); 4];

        assert_eq!(plan(&[e], &caps(2), PAGE, &mut out), Ok(1));
        assert_eq!(
            out[0],
            EntryPlan::Refuse(Refusal::Unheld(BindError::NoSuchCapability {
                slot: 0,
                index: 9,
                held: 2,
            })),
            "the three fields the loader prints come from the refusal itself"
        );
    }

    #[test]
    fn validate_refuses_before_bind_is_asked() {
        // The entry is *both* malformed and over-reaching. The order decides
        // which refusal the operator is shown, and it is the geometry — asked
        // before anything takes frames for a program that cannot fit.
        let mut e = entry("both", [Some(9), None, None, None]);
        e.text_pages = 0;
        let mut out = [EntryPlan::Spawn {
            index: 0,
            home_cpu: 0,
            slots: [None; MAX_SLOTS],
        }; 4];

        assert_eq!(plan(&[e], &caps(2), PAGE, &mut out), Ok(1));
        assert!(
            matches!(
                out[0],
                EntryPlan::Refuse(Refusal::Invalid(BindError::BadGeometry { .. }))
            ),
            "malformed wins over over-reaching, got {:?}",
            out[0]
        );
    }

    static BIG_IMAGE: [u8; PAGE + 1] = [0; PAGE + 1];

    #[test]
    fn an_image_larger_than_its_text_pages_is_invalid() {
        let mut e = entry("fat", [None; MAX_SLOTS]);
        e.text_pages = 1;
        e.image = &BIG_IMAGE;
        let mut out = [EntryPlan::Spawn {
            index: 0,
            home_cpu: 0,
            slots: [None; MAX_SLOTS],
        }; 4];

        assert_eq!(plan(&[e], &caps(2), PAGE, &mut out), Ok(1));
        assert!(matches!(
            out[0],
            EntryPlan::Refuse(Refusal::Invalid(BindError::ImageTooLarge { .. }))
        ));
    }

    #[test]
    fn a_home_outside_the_cpu_range_is_invalid() {
        let mut e = entry("nowhere", [None; MAX_SLOTS]);
        e.home_cpu = 9;
        let mut out = [EntryPlan::Spawn {
            index: 0,
            home_cpu: 0,
            slots: [None; MAX_SLOTS],
        }; 4];

        assert_eq!(plan(&[e], &caps(2), PAGE, &mut out), Ok(1));
        assert!(matches!(
            out[0],
            EntryPlan::Refuse(Refusal::Invalid(BindError::BadHome { .. }))
        ));
    }

    #[test]
    fn one_refusal_does_not_stop_the_entries_after_it() {
        // The loader reports and continues: a composition with one bad entry
        // still brings up the rest, which is what makes the refusal line
        // diagnostic rather than fatal.
        let bad = entry("bad", [Some(9), None, None, None]);
        let good = entry("good", [Some(0), None, None, None]);
        let mut out = [EntryPlan::Spawn {
            index: 0,
            home_cpu: 0,
            slots: [None; MAX_SLOTS],
        }; 4];

        assert_eq!(plan(&[bad, good], &caps(2), PAGE, &mut out), Ok(2));
        assert!(matches!(out[0], EntryPlan::Refuse(Refusal::Unheld(_))));
        match out[1] {
            EntryPlan::Spawn { index, slots, .. } => {
                assert_eq!(index, 1, "the index is the entry's, not the plan's");
                assert_eq!(slots[0], caps(2)[0]);
            }
            other => panic!("expected the second entry to be planned, got {other:?}"),
        }
    }

    #[test]
    fn the_source_says_which_manifest_the_plan_came_from() {
        // Carried beside the plan rather than inferred from its length: a store
        // of two and a builtin of two are the same table and different facts.
        assert_ne!(Source::Store { agents: 2 }, Source::Builtin);
        assert_eq!(Source::Store { agents: 2 }, Source::Store { agents: 2 });
    }
}
