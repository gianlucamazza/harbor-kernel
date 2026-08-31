//! Typed, conservative status facts consumed by device surfaces.
//!
//! A surface must never turn a missing probe into a positive claim.  Producers
//! publish the strongest fact they actually observed; renderers decide how to
//! present it without gaining authority over the underlying device.

/// Availability of a subsystem or capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    /// The subsystem was observed working.
    Ready,
    /// The subsystem is compiled but not present or not configured.
    Unavailable,
    /// A probe or operation failed.
    Error,
    /// No probe has supplied a fact yet.
    Unknown,
}

/// Conservative status of the display backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayFacts {
    pub health: Health,
    pub bit_hz: u32,
    pub clock_divisor: u32,
    pub frames: u32,
    pub flushes: u32,
    pub flush_errors: u32,
    pub bytes: u64,
}

/// Conservative status of the optional touch controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchFacts {
    pub health: Health,
    pub samples: u32,
    pub errors: u32,
    pub irq_low: bool,
}

/// Facts that are not allowed to be inferred from the display probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeripheralFacts {
    pub genet: Health,
    pub storage: Health,
}

/// Snapshot rendered by a status surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub display: DisplayFacts,
    pub touch: TouchFacts,
    pub peripherals: PeripheralFacts,
}

impl Snapshot {
    /// Facts before the corresponding producer has reported anything.
    pub const fn unknown() -> Self {
        Self {
            display: DisplayFacts {
                health: Health::Unknown,
                bit_hz: 0,
                clock_divisor: 0,
                frames: 0,
                flushes: 0,
                flush_errors: 0,
                bytes: 0,
            },
            touch: TouchFacts {
                health: Health::Unknown,
                samples: 0,
                errors: 0,
                irq_low: false,
            },
            peripherals: PeripheralFacts {
                genet: Health::Unknown,
                storage: Health::Unknown,
            },
        }
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::unknown()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_snapshot_makes_no_positive_claims() {
        let snapshot = Snapshot::unknown();
        assert_eq!(snapshot.display.health, Health::Unknown);
        assert_eq!(snapshot.touch.health, Health::Unknown);
        assert_eq!(snapshot.peripherals.genet, Health::Unknown);
        assert_eq!(snapshot.peripherals.storage, Health::Unknown);
    }
}
