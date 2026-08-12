//! The vocabulary a composition may name (ADR-0099) — pure, host-tested.
//!
//! A manifest entry names authority by **index** into the loader's `held` list
//! ([`crate::manifest`]), so the list is the whole of what any composed agent
//! can be given. This module is that list, and it exists for one property the
//! obvious version does not have: **an index means the same thing whether or
//! not the capability behind it was minted**.
//!
//! # Why declaring and providing are two calls
//!
//! The list used to be built from what succeeded:
//!
//! ```text
//! let held = [console?, blob?, …];   // whatever came up this boot
//! ```
//!
//! With one entry a failed mint gives an empty list, every agent is refused,
//! and the failure is loud. With four, a failed *first* mint shifts every later
//! index down by one: an entry asking for index 1 — console, to whoever composed
//! it — is bound to the storage endpoint instead, and runs. Nothing prints, the
//! loader reports `refusals=0`, and the arithmetic was correct on its own terms.
//!
//! So [`Set::declare`] reserves a position before anyone knows whether the mint
//! will succeed, and [`Set::provide`] fills it afterwards. A failure leaves a
//! **hole**, which [`crate::manifest::bind`] refuses as
//! [`BindError::HeldVacant`](crate::manifest::BindError::HeldVacant) — a
//! different fact from naming an index that was never declared, and a different
//! line on the console.
//!
//! The names are carried beside the capabilities because a vacancy has to be
//! printable: `authority: 1 blob VACANT` names the service that did not come
//! up, where an index alone would make a reader go and count.

use crate::agentstore::{SLOT_NONE, WINDOW_NONE};
use crate::cap::CapId;
use crate::paging::Perms;

/// Positions a composition's capability vocabulary may hold.
///
/// Longer than [`MAX_SLOTS`](crate::manifest::MAX_SLOTS) on purpose: the
/// vocabulary is what the *composition* may choose from, and the four slots are
/// the ceiling on what one agent may hold at once (ADR-0017).
pub const MAX_HELD: usize = 8;

/// Positions the device-window vocabulary may hold (ADR-0100).
///
/// Smaller than [`MAX_HELD`] only because a board has fewer pages worth giving
/// away than a product has services. Neither number is a security bound — the
/// bound is that an index outside the vocabulary is refused by arithmetic.
pub const MAX_WINDOWS: usize = 4;

/// A declared index must never collide with the store's sentinels.
const _: () = assert!(MAX_HELD < SLOT_NONE as usize);
const _: () = assert!(MAX_WINDOWS < WINDOW_NONE as usize);

/// A page of device memory the board is willing to hand to an agent (ADR-0100).
///
/// The physical address is here and **never on the wire**: a composition names
/// the position, not the page. That is the whole security argument — a store
/// that could carry a `pa` could name the kernel's own text and have it mapped
/// `USER_RW`, which is a mint, and this project removed that shape for
/// capabilities in ADR-0021 rather than guarding it with a range check.
///
/// `perms` travels with the page because a device is not always writable, and
/// the mapping site had `Perms::USER_RW` welded into it before this ADR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// Physical address of the device page. Comes from the BSP, via
    /// `bootstrap::authority` — rule 1 of `architecture.md` keeps it there.
    pub pa: u64,
    /// Rights the page is mapped with in the agent's window.
    pub perms: Perms,
}

/// The product's capability vocabulary (ADR-0099).
pub type Held = Set<CapId, MAX_HELD>;

/// The product's device-window vocabulary (ADR-0100).
pub type Windows = Set<Window, MAX_WINDOWS>;

/// Why a position could not be declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclareError {
    /// The vocabulary already holds [`MAX_HELD`] positions.
    Full { max: usize },
    /// A position with this name was already declared, at `index`.
    ///
    /// Two services under one name would make the index ambiguous in the one
    /// direction that matters: the packer resolves a name to an index.
    Duplicate { name: &'static str, index: u8 },
}

/// Why a position could not be provided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvideError {
    /// No position with that index has been declared.
    OutOfRange { index: u8, declared: usize },
    /// That position already holds something.
    ///
    /// Refused rather than overwritten: a second `provide` means two owners
    /// believe they mint the same authority, and whichever ran last would win
    /// silently.
    AlreadyProvided { index: u8, name: &'static str },
}

/// The declared vocabulary, and whatever has been provided into it.
///
/// Generic over what a position holds because ADR-0100 needed a second
/// vocabulary — device windows — with the *same* discipline, and the property
/// that earns the reuse is the one about holes: a position that fails to come
/// up must not move the ones after it, whether it holds a capability or a page
/// of MMIO. One mechanism, two alphabets ([`Held`] and [`Windows`]).
#[derive(Clone, Copy, Debug)]
pub struct Set<T, const N: usize = MAX_HELD> {
    items: [Option<T>; N],
    names: [&'static str; N],
    len: usize,
}

impl<T: Copy, const N: usize> Default for Set<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy, const N: usize> Set<T, N> {
    pub const fn new() -> Self {
        Self {
            // Inline const rather than `[None; N]`: the latter needs `T: Copy`
            // provable in a const context, which a generic parameter is not.
            items: [const { None }; N],
            names: [""; N],
            len: 0,
        }
    }

    /// Reserve the next position under `name`, minting nothing.
    ///
    /// The returned index is final: later declarations append, and a position
    /// that is never provided stays a hole rather than closing up.
    pub fn declare(&mut self, name: &'static str) -> Result<u8, DeclareError> {
        if let Some(index) = self.index_of(name) {
            return Err(DeclareError::Duplicate { name, index });
        }
        if self.len == N {
            return Err(DeclareError::Full { max: N });
        }
        let index = self.len as u8;
        self.names[self.len] = name;
        self.len += 1;
        Ok(index)
    }

    /// Fill a declared position with what was minted or found for it.
    pub fn provide(&mut self, index: u8, item: T) -> Result<(), ProvideError> {
        let i = index as usize;
        if i >= self.len {
            return Err(ProvideError::OutOfRange {
                index,
                declared: self.len,
            });
        }
        if self.items[i].is_some() {
            return Err(ProvideError::AlreadyProvided {
                index,
                name: self.names[i],
            });
        }
        self.items[i] = Some(item);
        Ok(())
    }

    /// What [`crate::manifest::bind`] indexes: one entry per declared position,
    /// `None` where nothing was provided.
    #[inline]
    pub fn as_slice(&self) -> &[Option<T>] {
        &self.items[..self.len]
    }

    /// How many positions have been declared.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The name of a declared position, for the boot line and for refusals.
    #[inline]
    pub fn name_of(&self, index: u8) -> Option<&'static str> {
        let i = index as usize;
        (i < self.len).then(|| self.names[i])
    }

    /// The index a name was declared at, if any.
    ///
    /// The direction the packer needs: a composition names `console`, the
    /// kernel says which integer that is.
    pub fn index_of(&self, name: &str) -> Option<u8> {
        self.names[..self.len]
            .iter()
            .position(|n| *n == name)
            .map(|i| i as u8)
    }

    /// Whether a declared position was ever provided.
    #[inline]
    pub fn is_provided(&self, index: u8) -> bool {
        self.get(index).is_some()
    }

    /// What sits at a declared position, if anything was provided.
    ///
    /// For the kernel side, which sometimes needs something it declared for its
    /// own use rather than for a composition — the oracle's demos hold the
    /// console end that way. An undeclared index and a declared-but-empty one
    /// both answer `None`: the caller here is asking *can I use it*, and the
    /// distinction between the two belongs to [`crate::manifest::bind`], where
    /// it changes which refusal a composition is told.
    #[inline]
    pub fn get(&self, index: u8) -> Option<T> {
        let i = index as usize;
        if i < self.len { self.items[i] } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(n: u16) -> CapId {
        CapId::new(n, 1)
    }

    #[test]
    fn a_vacancy_at_zero_does_not_move_one() {
        // The bug this module exists for. Console fails to mint, blob succeeds,
        // and an entry naming index 1 must still reach blob — not console's
        // former place, and not blob shifted down into index 0.
        let mut set = Held::new();
        let console = set.declare("console").unwrap();
        let blob = set.declare("blob").unwrap();
        assert_eq!((console, blob), (0, 1));

        // Console's mint failed: nothing is provided at 0.
        set.provide(blob, cap(7)).unwrap();

        assert_eq!(set.as_slice(), &[None, Some(cap(7))]);
        assert_eq!(set.len(), 2, "a hole still occupies its position");
        assert!(!set.is_provided(console));
        assert!(set.is_provided(blob));
    }

    #[test]
    fn declaring_returns_positions_in_order_and_names_them() {
        let mut set = Held::new();
        assert_eq!(set.declare("console").unwrap(), 0);
        assert_eq!(set.declare("blob").unwrap(), 1);
        assert_eq!(set.name_of(0), Some("console"));
        assert_eq!(set.name_of(1), Some("blob"));
        assert_eq!(set.name_of(2), None, "not declared, so not named");
        assert_eq!(set.index_of("blob"), Some(1));
        assert_eq!(set.index_of("uart0"), None);
    }

    #[test]
    fn a_duplicate_name_is_refused_and_names_the_position_that_holds_it() {
        // The packer resolves a name to an index; two positions under one name
        // would make that lookup ambiguous in exactly the direction used.
        let mut set = Held::new();
        set.declare("console").unwrap();
        set.declare("blob").unwrap();
        assert_eq!(
            set.declare("console"),
            Err(DeclareError::Duplicate {
                name: "console",
                index: 0
            })
        );
        assert_eq!(set.len(), 2, "a refused declaration reserves nothing");
    }

    #[test]
    fn the_last_position_declares_and_the_next_one_does_not() {
        let mut set = Held::new();
        // Distinct names, since Duplicate is checked first.
        const NAMES: [&str; MAX_HELD] = ["a", "b", "c", "d", "e", "f", "g", "h"];
        for (i, name) in NAMES.iter().enumerate() {
            assert_eq!(set.declare(name).unwrap(), i as u8);
        }
        assert_eq!(
            set.declare("over"),
            Err(DeclareError::Full { max: MAX_HELD })
        );
        assert_eq!(set.len(), MAX_HELD);
    }

    #[test]
    fn providing_an_undeclared_position_is_refused() {
        let mut set = Held::new();
        let console = set.declare("console").unwrap();
        assert_eq!(
            set.provide(1, cap(3)),
            Err(ProvideError::OutOfRange {
                index: 1,
                declared: 1
            }),
            "the position after the last declared one"
        );
        set.provide(console, cap(3)).unwrap();
        assert_eq!(set.as_slice(), &[Some(cap(3))]);
    }

    #[test]
    fn providing_twice_is_refused_rather_than_overwritten() {
        // Two owners minting one authority: the last writer would win in
        // silence, and which one ran last is not a thing anyone reasons about.
        let mut set = Held::new();
        let console = set.declare("console").unwrap();
        set.provide(console, cap(1)).unwrap();
        assert_eq!(
            set.provide(console, cap(2)),
            Err(ProvideError::AlreadyProvided {
                index: 0,
                name: "console"
            })
        );
        assert_eq!(
            set.as_slice(),
            &[Some(cap(1))],
            "the first capability stands"
        );
    }

    #[test]
    fn get_answers_none_for_a_hole_and_for_a_position_that_was_never_declared() {
        // Both are "you cannot use this", and the caller that asks is the kernel
        // using its own service. Which of the two it is only matters to `bind`,
        // where it decides what a composition is told.
        let mut set = Held::new();
        let console = set.declare("console").unwrap();
        let blob = set.declare("blob").unwrap();
        set.provide(blob, cap(4)).unwrap();

        assert_eq!(set.get(console), None, "declared, never minted");
        assert_eq!(set.get(blob), Some(cap(4)));
        assert_eq!(set.get(2), None, "never declared");
    }

    #[test]
    fn an_empty_vocabulary_binds_nothing() {
        let set = Held::new();
        assert!(set.is_empty());
        assert_eq!(set.as_slice(), &[]);
        assert!(!set.is_provided(0));
    }

    // ---------------------------------------------------------------------
    // ADR-0100: the same mechanism, holding pages instead of capabilities.
    // ---------------------------------------------------------------------

    fn window(pa: u64) -> Window {
        Window {
            pa,
            perms: Perms::USER_RW,
        }
    }

    #[test]
    fn a_window_vacancy_at_zero_does_not_move_one_either() {
        // The property the reuse is for. A device that is not on this board
        // leaves its position empty; the window declared after it keeps its
        // index, so a composition naming 1 gets 1 — and never index 0's page,
        // which is a page of MMIO it was not composed to touch.
        let mut set = Windows::new();
        let rng = set.declare("rng").unwrap();
        let uart = set.declare("uart0").unwrap();
        assert_eq!((rng, uart), (0, 1));

        set.provide(uart, window(0xfe20_1000)).unwrap();

        assert_eq!(set.as_slice(), &[None, Some(window(0xfe20_1000))]);
        assert!(!set.is_provided(rng), "absent on this board");
        assert_eq!(set.get(uart).map(|w| w.pa), Some(0xfe20_1000));
    }

    #[test]
    fn the_window_vocabulary_has_its_own_ceiling() {
        // Same refusal, different bound: MAX_WINDOWS, not MAX_HELD. A shared
        // ceiling would let a board with many devices eat the product's
        // capability positions, which are a different resource entirely.
        let mut set = Windows::new();
        const NAMES: [&str; MAX_WINDOWS] = ["a", "b", "c", "d"];
        for (i, name) in NAMES.iter().enumerate() {
            assert_eq!(set.declare(name).unwrap(), i as u8);
        }
        // `max: MAX_WINDOWS` rather than `MAX_HELD` is the whole assertion: the
        // refusal reports the window vocabulary's own ceiling, so a board with
        // many devices cannot eat the product's capability positions.
        assert_eq!(
            set.declare("over"),
            Err(DeclareError::Full { max: MAX_WINDOWS })
        );
    }

    #[test]
    fn a_window_carries_its_own_rights() {
        // `Perms::USER_RW` was welded into the mapping site before ADR-0100.
        // A read-only device page is now expressible, and the vocabulary is
        // what expresses it.
        let mut set = Windows::new();
        let counter = set.declare("counter").unwrap();
        set.provide(
            counter,
            Window {
                pa: 0xfe00_3000,
                perms: Perms::USER_RO,
            },
        )
        .unwrap();
        assert_eq!(set.get(counter).map(|w| w.perms), Some(Perms::USER_RO));
    }

    #[test]
    fn providing_a_window_twice_is_refused_like_a_capability() {
        // The discipline is the mechanism's, not the capability's: two owners
        // claiming one position is the same mistake whatever the position holds.
        let mut set = Windows::new();
        let rng = set.declare("rng").unwrap();
        set.provide(rng, window(0xfe10_4000)).unwrap();
        assert_eq!(
            set.provide(rng, window(0xdead_0000)),
            Err(ProvideError::AlreadyProvided {
                index: 0,
                name: "rng"
            })
        );
        assert_eq!(
            set.get(rng).map(|w| w.pa),
            Some(0xfe10_4000),
            "the first window stands"
        );
    }
}
