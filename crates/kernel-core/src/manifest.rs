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
use crate::held::Window;
use crate::paging::Perms;

/// Capability slots an agent may be given.
///
/// Must equal `sched::MAX_CAPS_PER_TASK`. The compile-time assertion lives in
/// `src/bootstrap/loader.rs`, the layer that binds the two: the scheduler owns
/// the array, but it has no business naming a manifest to state its own bound
/// (ADR-0023).
pub const MAX_SLOTS: usize = 4;
/// Capability indices at and above this boundary belong to the network
/// packet-pool vocabulary and require an explicit packet-pool grant.
pub const PACKET_CAPABILITY_START: u8 = 3;

/// One page of MMIO an agent is allowed to reach.
///
/// A page, singular, named by **where** it lands and **which** declared window
/// it is (ADR-0100).
///
/// The split is the security argument. The virtual address is the composition's
/// to choose because it is the composition's own window; the physical address
/// is not here at all, because an entry that could name one could name the
/// kernel's text and have it mapped `USER_RW`. The index is resolved against
/// [`crate::held::Windows`], which `bootstrap::authority` fills from the BSP —
/// keeping the board's map in the one layer allowed to know it (ADR-0013's
/// narrow windows, `architecture.md` rule 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceGrant {
    /// Virtual address inside the agent's own window.
    pub va: u64,
    /// Index into the loader's window vocabulary.
    pub window: u8,
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
    /// Permit this agent to use the non-ambient `SYS_RESOLVE` grant (ADR-0102).
    pub may_resolve: bool,
    /// One optional device page.
    pub device: Option<DeviceGrant>,
    /// Grant the fixed packet-pool mapping owned by the EL1 network service.
    /// The physical pages are never present in the manifest or on the wire.
    pub packet_pool: bool,
    /// Sticky home CPU for the EL1 driver task ([ADR-0088](../../../docs/adr/0088-product-home-cpu.md)).
    ///
    /// Domain is `0 .. crate::tasks::N_CPUS`. Default product home is **0**;
    /// a store may pin an agent to another online core without ambient spread.
    pub home_cpu: u8,
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
    /// The entry named a position that exists in the vocabulary and holds
    /// nothing (ADR-0099).
    ///
    /// Not the same refusal as [`Self::NoSuchCapability`], and deliberately so:
    /// this one says a service the composition was entitled to expect did not
    /// come up this boot. The position's *name* is not here because this module
    /// binds against a slice of capabilities and has no names to give — the
    /// vocabulary ([`crate::held::Set::name_of`]) supplies it where the refusal
    /// is printed.
    HeldVacant { slot: usize, index: u8 },
    /// A network capability was named without the corresponding pool mapping.
    PacketPoolRequired { slot: usize, index: u8 },
    /// The entry named a device window past the end of the vocabulary
    /// (ADR-0100).
    ///
    /// The device half of [`Self::NoSuchCapability`], and arithmetic for the
    /// same reason: `index >= windows.len()`. A composition cannot reach a page
    /// the board did not declare, and cannot be given one by a check that was
    /// forgotten, because there is no check.
    NoSuchWindow { index: u8, windows: usize },
    /// The entry named a declared window that holds nothing this boot
    /// (ADR-0100) — the device is absent on this board.
    WindowVacant { index: u8 },
    /// The image does not fit in the text pages the entry declared.
    ImageTooLarge { bytes: usize, capacity: usize },
    /// Zero text pages, or a window with no stack.
    BadGeometry {
        text_pages: usize,
        stack_pages: usize,
    },
    /// `home_cpu` outside `0 .. N_CPUS` ([ADR-0088](../../../docs/adr/0088-product-home-cpu.md)).
    BadHome { home_cpu: u8, max: u8 },
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
        let max = crate::tasks::N_CPUS as u8;
        if self.home_cpu >= max {
            return Err(BindError::BadHome {
                home_cpu: self.home_cpu,
                max,
            });
        }
        Ok(())
    }
}

/// Turn an entry's slot indices into the capability table a task is spawned with.
///
/// `held` is the loader's **vocabulary** ([`crate::held`]): one position per
/// declared authority, `None` where nothing was minted this boot. Every slot is
/// an index into it, so the result can only contain capabilities the loader
/// already had — which is the whole security argument of the manifest, and it is
/// arithmetic rather than a check that could be forgotten.
///
/// The two refusals are different facts (ADR-0099).
/// [`BindError::NoSuchCapability`] means the entry named a position that does
/// not exist; [`BindError::HeldVacant`] means it named one that does and that
/// nobody filled — a boot-time failure of the kernel rather than a
/// mis-composition. Collapsing them would make a service that failed to start
/// read like a manifest that asked for too much.
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
///     may_resolve: false,
///     device: None,
///     packet_pool: false,
///     home_cpu: 0,
/// };
/// let held = [Some(CapId::new(7, 1)), Some(CapId::new(8, 1))];
/// assert_eq!(bind(&entry, &held).unwrap()[0], Some(CapId::new(8, 1)));
///
/// let short = [Some(CapId::new(7, 1))];
/// assert_eq!(
///     bind(&entry, &short),
///     Err(BindError::NoSuchCapability { slot: 0, index: 1, held: 1 })
/// );
///
/// // Declared, never minted: the position is there and empty.
/// let vacant = [Some(CapId::new(7, 1)), None];
/// assert_eq!(
///     bind(&entry, &vacant),
///     Err(BindError::HeldVacant { slot: 0, index: 1 })
/// );
/// ```
pub fn bind(
    entry: &AgentEntry,
    held: &[Option<CapId>],
) -> Result<[Option<CapId>; MAX_SLOTS], BindError> {
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
        if index >= PACKET_CAPABILITY_START && !entry.packet_pool {
            return Err(BindError::PacketPoolRequired { slot, index });
        }
        let Some(cap) = held[i] else {
            return Err(BindError::HeldVacant { slot, index });
        };
        out[slot] = Some(cap);
    }
    Ok(out)
}

/// A device page the loader may now map: where the composition asked for it,
/// and what the board says it is (ADR-0100).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedWindow {
    /// From the entry — inside the agent's own window.
    pub va: u64,
    /// From the vocabulary — never from the entry.
    pub pa: u64,
    /// From the vocabulary, so a read-only device stays read-only.
    pub perms: Perms,
}

/// Resolve an entry's device grant against the loader's window vocabulary.
///
/// The companion to [`bind`], and the same argument one layer over: an entry
/// carries an **index**, so the physical address it ends up with can only be
/// one the board declared. `Ok(None)` means the entry asked for no device,
/// which is what every entry in the tree does today.
///
/// ```
/// use kernel_core::held::Window;
/// use kernel_core::manifest::{bind_window, AgentEntry, BindError, DeviceGrant};
/// use kernel_core::paging::Perms;
///
/// const IMAGE: [u8; 4] = [0; 4];
/// let mut entry = AgentEntry {
///     name: "rng-agent",
///     image: &IMAGE,
///     text_pages: 1,
///     stack_pages: 3,
///     slots: [None; 4],
///     may_resolve: false,
///     device: Some(DeviceGrant { va: 0x2000, window: 0 }),
///     packet_pool: false,
///     home_cpu: 0,
/// };
/// let windows = [Some(Window { pa: 0xfe10_4000, perms: Perms::USER_RW })];
/// let got = bind_window(&entry, &windows).unwrap().unwrap();
/// assert_eq!((got.va, got.pa), (0x2000, 0xfe10_4000));
///
/// // Past the end of the vocabulary: arithmetic, not a range check.
/// entry.device = Some(DeviceGrant { va: 0x2000, window: 1 });
/// assert_eq!(
///     bind_window(&entry, &windows),
///     Err(BindError::NoSuchWindow { index: 1, windows: 1 })
/// );
///
/// // Declared, absent on this board.
/// assert_eq!(
///     bind_window(&entry, &[Some(Window { pa: 0, perms: Perms::USER_RW }), None]),
///     Err(BindError::WindowVacant { index: 1 })
/// );
/// ```
pub fn bind_window(
    entry: &AgentEntry,
    windows: &[Option<Window>],
) -> Result<Option<ResolvedWindow>, BindError> {
    let Some(grant) = entry.device else {
        return Ok(None);
    };
    let i = grant.window as usize;
    if i >= windows.len() {
        return Err(BindError::NoSuchWindow {
            index: grant.window,
            windows: windows.len(),
        });
    }
    let Some(window) = windows[i] else {
        return Err(BindError::WindowVacant {
            index: grant.window,
        });
    };
    Ok(Some(ResolvedWindow {
        va: grant.va,
        pa: window.pa,
        perms: window.perms,
    }))
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
            may_resolve: false,
            device: None,
            packet_pool: false,
            home_cpu: 0,
        }
    }

    #[test]
    fn home_cpu_out_of_range_is_refused() {
        let mut e = entry([None; MAX_SLOTS]);
        e.home_cpu = crate::tasks::N_CPUS as u8;
        assert_eq!(
            e.validate(4096),
            Err(BindError::BadHome {
                home_cpu: crate::tasks::N_CPUS as u8,
                max: crate::tasks::N_CPUS as u8,
            })
        );
    }

    #[test]
    fn an_index_the_loader_does_not_hold_is_refused_rather_than_wrapped() {
        // The assertion the manifest exists for. Index 9 against two held
        // capabilities is not a panic, not a silent `None`, and above all not a
        // read past the end of the loader's list — it is a refusal that names
        // the slot, the index, and how many the loader actually had.
        let held = [Some(CapId::new(1, 1)), Some(CapId::new(2, 1))];
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
        let held = [Some(CapId::new(1, 1)), Some(CapId::new(2, 1))];
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
        let held = [Some(CapId::new(1, 1))];
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
        let held = [Some(CapId::new(5, 2)), Some(CapId::new(6, 2))];
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
    fn a_declared_position_that_holds_nothing_is_a_different_refusal() {
        // ADR-0099. Index 1 exists in the vocabulary and is empty, because the
        // service behind it failed to start. Reporting NoSuchCapability here
        // would tell whoever reads the console that the composition asked for
        // too much, when what happened is that the kernel came up short.
        let held = [Some(CapId::new(1, 1)), None];
        assert_eq!(
            bind(&entry([Some(1), None, None, None]), &held),
            Err(BindError::HeldVacant { slot: 0, index: 1 })
        );
        assert_ne!(
            bind(&entry([Some(1), None, None, None]), &held),
            Err(BindError::NoSuchCapability {
                slot: 0,
                index: 1,
                held: 2
            }),
            "the two refusals must not collapse into one"
        );
    }

    #[test]
    fn a_vacancy_does_not_shift_the_positions_after_it() {
        // The whole point of a vocabulary with holes: index 1 still reaches the
        // capability declared at 1, with 0 empty. A list built from what was
        // minted would have handed slot 0's grant to whoever asked for 1.
        let held = [None, Some(CapId::new(6, 2))];
        assert_eq!(
            bind(&entry([None, Some(1), None, None]), &held).unwrap(),
            [None, Some(CapId::new(6, 2)), None, None]
        );
        assert_eq!(
            bind(&entry([Some(0), None, None, None]), &held),
            Err(BindError::HeldVacant { slot: 0, index: 0 }),
            "and the hole itself is still refused"
        );
    }

    #[test]
    fn a_window_past_the_vocabulary_is_refused_by_arithmetic() {
        // ADR-0100's security property, stated as a test: the entry names an
        // index, so the only physical addresses reachable are the ones the
        // board declared. There is no check to forget, and no range to widen.
        let mut e = entry([None; MAX_SLOTS]);
        e.device = Some(DeviceGrant {
            va: 0x2000,
            window: 1,
        });
        let windows = [Some(Window {
            pa: 0xfe10_4000,
            perms: Perms::USER_RW,
        })];
        assert_eq!(
            bind_window(&e, &windows),
            Err(BindError::NoSuchWindow {
                index: 1,
                windows: 1
            })
        );
        // And an empty vocabulary refuses everything, which is what a product
        // that declares no window is entitled to.
        assert_eq!(
            bind_window(&e, &[]),
            Err(BindError::NoSuchWindow {
                index: 1,
                windows: 0
            })
        );
    }

    #[test]
    fn an_absent_device_is_a_different_refusal_from_an_undeclared_one() {
        // Same distinction ADR-0099 drew for capabilities, for the same reason:
        // "this board does not have that device" and "your composition asked
        // for a device nobody declared" are different problems with different
        // owners, and one console line each.
        let mut e = entry([None; MAX_SLOTS]);
        e.device = Some(DeviceGrant {
            va: 0x2000,
            window: 0,
        });
        assert_eq!(
            bind_window(&e, &[None]),
            Err(BindError::WindowVacant { index: 0 })
        );
        assert_ne!(
            bind_window(&e, &[None]),
            Err(BindError::NoSuchWindow {
                index: 0,
                windows: 1
            }),
            "the two refusals must not collapse into one"
        );
    }

    #[test]
    fn the_physical_address_comes_from_the_board_and_the_va_from_the_entry() {
        // Each half from the party entitled to decide it. A composition that
        // could choose the pa would be minting memory; one that could not
        // choose the va could not lay out its own window.
        let mut e = entry([None; MAX_SLOTS]);
        e.device = Some(DeviceGrant {
            va: 0x9000,
            window: 1,
        });
        let windows = [
            Some(Window {
                pa: 0xfe20_1000,
                perms: Perms::USER_RW,
            }),
            Some(Window {
                pa: 0xfe10_4000,
                perms: Perms::USER_RO,
            }),
        ];
        assert_eq!(
            bind_window(&e, &windows).unwrap(),
            Some(ResolvedWindow {
                va: 0x9000,
                pa: 0xfe10_4000,
                perms: Perms::USER_RO,
            }),
            "index 1's page and index 1's rights, not index 0's"
        );
    }

    #[test]
    fn an_entry_that_names_no_device_resolves_to_nothing() {
        // Denied by default, the device half: an agent with no grant is not an
        // agent with the first window.
        let e = entry([None; MAX_SLOTS]);
        assert_eq!(
            bind_window(
                &e,
                &[Some(Window {
                    pa: 0xfe10_4000,
                    perms: Perms::USER_RW
                })]
            ),
            Ok(None)
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
