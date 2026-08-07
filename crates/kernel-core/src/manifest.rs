//! The table that says which agents exist and what authority each is given
//! (ADR-0021) — pure, host-tested.
//!
//! # A binding, not a mint
//!
//! An entry cannot name a capability. It names an **index into the loader's own
//! list of capabilities**, and [`bind`] turns that index into a [`CapId`] by
//! indexing. The property is the same one [`crate::cap::from_slot`] has a floor
//! below, and it is a property of the shape rather than of a check: there is
//! nothing outside the loader's list for an entry to reach, so a manifest cannot
//! escalate no matter what it says.
//!
//! That is why the field is `Option<u8>` and not `Option<CapId>`. A `CapId` in a
//! manifest would be a mint — a table that could grant authority nobody handed
//! it — and the whole of [ADR-0021](../../../docs/adr/0021-agents-as-data-and-the-manifest.md)
//! turns on that distinction.
//!
//! # What is deliberately not here
//!
//! No parser, no version field, no byte format. The manifest is a Rust `const`
//! table, so it changes with the code that reads it, and its images are `const`
//! byte arrays in `.rodata` produced by [`crate::prog`] — checked byte-for-byte
//! against `llvm-mc`. A parser becomes worth its risk the day the bytes come
//! from outside the image, and not before (ADR-0021 §4).

use crate::cap::CapId;

/// Capability slots an agent may be given.
///
/// Must equal `sched::MAX_CAPS_PER_TASK`. The compile-time assertion lives in
/// `src/bootstrap/loader.rs`, the layer that binds the two: the scheduler owns
/// the array, but it has no business naming a manifest to state its own bound
/// (ADR-0023).
pub const MAX_SLOTS: usize = 4;

/// One page of MMIO an agent is allowed to reach.
///
/// A page, singular, and named by both addresses: the manifest lives in
/// `bootstrap`, which is the only layer permitted to know a board's physical
/// map (ADR-0013's narrow windows, `architecture.md` rule 3). The kernel maps
/// what the entry says and nothing adjacent to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceGrant {
    /// Virtual address inside the agent's own window.
    pub va: u64,
    /// Physical address of the device page.
    pub pa: u64,
}

/// One agent, as data.
#[derive(Clone, Copy, Debug)]
pub struct AgentEntry {
    /// What the boot log calls it. Not an identity — there is no namespace and
    /// nothing resolves it — but a log line naming `pl011` beats one naming
    /// entry 3.
    pub name: &'static str,
    /// The flat image, mapped at the window base and entered at offset 0.
    pub image: &'static [u8],
    /// Executable pages. The image must fit inside them.
    pub text_pages: usize,
    /// Writable pages above the text, ending at the initial `SP`.
    pub stack_pages: usize,
    /// Slot `i` holds the capability at index `slots[i]` of the loader's list.
    pub slots: [Option<u8>; MAX_SLOTS],
    /// One optional device page.
    pub device: Option<DeviceGrant>,
}

/// Why an entry could not be turned into a task's capability table or a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    /// The entry named a capability the loader does not hold.
    ///
    /// The refusal this type exists for. It is arithmetic — `index >= held.len()`
    /// — rather than a policy decision, which is the point: an entry that could
    /// name authority outside the loader's list would be a mint.
    NoSuchCapability { slot: usize, index: u8, held: usize },
    /// The image does not fit in the text pages the entry declared.
    ImageTooLarge { bytes: usize, capacity: usize },
    /// Zero text pages, or a window with no stack.
    BadGeometry {
        text_pages: usize,
        stack_pages: usize,
    },
}

impl AgentEntry {
    /// Total pages of the agent's window: text then stack.
    #[inline]
    #[must_use]
    pub const fn total_pages(&self) -> usize {
        self.text_pages + self.stack_pages
    }

    /// Check the entry against a frame size, before anything is allocated.
    ///
    /// Separate from [`bind`] because it answers a different question — *is this
    /// entry self-consistent* rather than *may this loader grant it* — and
    /// because a caller wants the geometry refused before it takes frames from
    /// the pool for a program that will not fit in them.
    pub const fn validate(&self, frame_size: usize) -> Result<(), BindError> {
        if self.text_pages == 0 || self.stack_pages == 0 {
            return Err(BindError::BadGeometry {
                text_pages: self.text_pages,
                stack_pages: self.stack_pages,
            });
        }
        let capacity = self.text_pages * frame_size;
        if self.image.len() > capacity {
            return Err(BindError::ImageTooLarge {
                bytes: self.image.len(),
                capacity,
            });
        }
        Ok(())
    }
}

/// Turn an entry's slot indices into the capability table a task is spawned with.
///
/// `held` is what the loader itself holds. Every slot is an index into it, so
/// the result can only contain capabilities the loader already had — which is
/// the whole security argument of the manifest, and it is arithmetic rather than
/// a check that could be forgotten.
///
/// ```
/// use kernel_core::cap::CapId;
/// use kernel_core::manifest::{bind, AgentEntry, BindError, MAX_SLOTS};
///
/// const IMAGE: [u8; 4] = [0; 4];
/// let entry = AgentEntry {
///     name: "demo",
///     image: &IMAGE,
///     text_pages: 1,
///     stack_pages: 3,
///     slots: [Some(1), None, None, None],
///     device: None,
/// };
/// let held = [CapId::new(7, 1), CapId::new(8, 1)];
/// assert_eq!(bind(&entry, &held).unwrap()[0], Some(CapId::new(8, 1)));
///
/// let short = [CapId::new(7, 1)];
/// assert_eq!(
///     bind(&entry, &short),
///     Err(BindError::NoSuchCapability { slot: 0, index: 1, held: 1 })
/// );
/// ```
pub fn bind(entry: &AgentEntry, held: &[CapId]) -> Result<[Option<CapId>; MAX_SLOTS], BindError> {
    let mut out = [None; MAX_SLOTS];
    // `enumerate` rather than a hand-rolled counter, and that is a mutation
    // result rather than a style preference: the `slot += 1` this replaced
    // mutated to `slot *= 1`, which pins the index at zero and hangs the suite.
    // Cargo-mutants reports that as a *timeout* — detected, since a hanging test
    // is not a passing one, but detected by taking a minute instead of by an
    // assertion. An iterator has no counter to mutate.
    for (slot, granted) in entry.slots.iter().enumerate() {
        let Some(index) = *granted else { continue };
        let i = index as usize;
        if i >= held.len() {
            return Err(BindError::NoSuchCapability {
                slot,
                index,
                held: held.len(),
            });
        }
        out[slot] = Some(held[i]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: [u8; 16] = [0xaa; 16];

    fn entry(slots: [Option<u8>; MAX_SLOTS]) -> AgentEntry {
        AgentEntry {
            name: "t",
            image: &IMAGE,
            text_pages: 1,
            stack_pages: 3,
            slots,
            device: None,
        }
    }

    #[test]
    fn an_index_the_loader_does_not_hold_is_refused_rather_than_wrapped() {
        // The assertion the manifest exists for. Index 9 against two held
        // capabilities is not a panic, not a silent `None`, and above all not a
        // read past the end of the loader's list — it is a refusal that names
        // the slot, the index, and how many the loader actually had.
        let held = [CapId::new(1, 1), CapId::new(2, 1)];
        assert_eq!(
            bind(&entry([Some(9), None, None, None]), &held),
            Err(BindError::NoSuchCapability {
                slot: 0,
                index: 9,
                held: 2
            })
        );
    }

    #[test]
    fn the_last_held_index_binds_and_the_next_one_does_not() {
        // The off-by-one, on the other side of the same boundary `from_slot`
        // guards: `index >= len` and `index > len` differ only here.
        let held = [CapId::new(1, 1), CapId::new(2, 1)];
        assert_eq!(
            bind(&entry([Some(1), None, None, None]), &held).unwrap()[0],
            Some(CapId::new(2, 1))
        );
        assert!(bind(&entry([Some(2), None, None, None]), &held).is_err());
    }

    #[test]
    fn an_entry_that_names_nothing_gets_nothing() {
        // Denied by default (ADR-0017 §3): an agent with no slots is not an
        // agent with the loader's own authority.
        let held = [CapId::new(1, 1)];
        assert_eq!(
            bind(&entry([None; MAX_SLOTS]), &held),
            Ok([None; MAX_SLOTS])
        );
    }

    #[test]
    fn slots_keep_their_positions_including_the_holes() {
        // Slot 0 empty and slot 1 filled is the convention the demo agents use,
        // so that a program which miscounts finds nothing rather than something
        // adjacent. A bind that compacted the array would silently repair the
        // miscount and destroy the property.
        let held = [CapId::new(5, 2), CapId::new(6, 2)];
        let bound = bind(&entry([None, Some(0), None, Some(1)]), &held).unwrap();
        assert_eq!(
            bound,
            [None, Some(CapId::new(5, 2)), None, Some(CapId::new(6, 2))]
        );
    }

    #[test]
    fn an_empty_loader_can_grant_nothing_and_says_so() {
        assert_eq!(
            bind(&entry([Some(0), None, None, None]), &[]),
            Err(BindError::NoSuchCapability {
                slot: 0,
                index: 0,
                held: 0
            })
        );
    }

    #[test]
    fn geometry_is_refused_before_a_frame_is_taken() {
        let mut e = entry([None; MAX_SLOTS]);
        assert_eq!(e.validate(4096), Ok(()));

        e.text_pages = 0;
        assert_eq!(
            e.validate(4096),
            Err(BindError::BadGeometry {
                text_pages: 0,
                stack_pages: 3
            })
        );

        e.text_pages = 1;
        e.stack_pages = 0;
        assert!(
            e.validate(4096).is_err(),
            "a window with no stack is not a window"
        );
    }

    #[test]
    fn an_image_larger_than_its_declared_text_is_refused() {
        // The bound `poke_user` enforces at write time, checked here at load
        // time so the refusal names the entry instead of a page.
        let e = entry([None; MAX_SLOTS]);
        assert_eq!(
            e.validate(8),
            Err(BindError::ImageTooLarge {
                bytes: 16,
                capacity: 8
            })
        );
        assert_eq!(
            e.validate(16),
            Ok(()),
            "exactly filling the text is not too large"
        );
    }

    #[test]
    fn total_pages_is_text_plus_stack() {
        assert_eq!(entry([None; MAX_SLOTS]).total_pages(), 4);
    }
}
