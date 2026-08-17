//! The trace a boot leaves that does not pass through the serial line (ADR-0111).
//!
//! # Why this is not in `demos.rs`
//!
//! It was, and that was the bug. The oracle image initialised SDHCI, advanced a
//! `boot` counter in the durable region and committed it to the type `0x7f`
//! partition; `make product-builds` stripped all of it, so the image that ships
//! had no witness at all and the image that had one was not the image under
//! test.
//!
//! On 2026-08-17 that cost an afternoon. A USB-serial adapter dropped off the
//! bus four times in an hour, and a real regression — a deploy that removed the
//! board's device tree — produced the identical artifact: a zero-byte capture.
//! Nothing could tell the two apart, and a measurement designed on the
//! assumption that the product wrote the store produced a number that could not
//! move, from which "the board is not booting" was asserted as fact.
//!
//! # What it proves, and what it does not
//!
//! The counter advancing proves the board **reached this point**: firmware
//! handed off, the image was entered, the MMU came up, and the SD stack
//! answered. It does not prove bring-up completed — `make hw-check` over a
//! transcript is what judges a boot as a whole. Read the number as *"the
//! previous boot got this far"*.
//!
//! # Shape
//!
//! [`open`] does everything that must happen early: bring up the card, find the
//! store, load the winning A/B slot, restore it into the in-memory region,
//! advance the counter and print the cross-boot line. [`Witness::commit`]
//! writes it back.
//!
//! The product calls both immediately: one write, as early as the media stack
//! allows, because a witness written at the end of bring-up cannot witness
//! anything that stops bring-up. The oracle calls [`open`] at the same point
//! and [`Witness::commit`] after its demos have put their keys, so a single
//! flush carries both. One implementation, two call sites — the store that P2's
//! hardware claims rest on must not be written by a second copy of this
//! (ADR-0110).
//!
//! Every degraded path is one honest line and the boot proceeds on the
//! DRAM-only store (ADR-0045). A witness that can stop the product from booting
//! would turn an observation aid into a failure mode.

use crate::drivers::sdhci::{SdError, Sdhci};
use kernel_core::durable_media::Slot;

/// A loaded store on media, waiting to be written back.
pub struct Witness {
    sd: Sdhci,
    lba: u32,
    winner: Option<(Slot, u64)>,
}

/// Bring up media, load the store, advance the boot counter, and report.
///
/// Returns `None` on every degraded path, having said which one on the console.
pub fn open() -> Option<Witness> {
    use kernel_core::mbr;

    // SAFETY: exclusive SDHCI windows; core 0 only, before any agent runs.
    let (sd, host) = match unsafe { crate::bsp::board::sdhci::init() } {
        Ok(pair) => pair,
        Err(SdError::NotPresent) => {
            crate::kprintln!("durable-media: absent (NotPresent)");
            return None;
        }
        Err(SdError::NoCard) => {
            crate::kprintln!("durable-media: no-card (no SDHC/SDXC answered)");
            return None;
        }
        Err(SdError::Unsupported) => {
            crate::kprintln!("durable-media: unsupported (not SDHC/SDXC)");
            return None;
        }
        Err(e) => {
            crate::kprintln!("durable-media: error (init {e:?})");
            return None;
        }
    };

    let mut sector0 = [0u8; 512];
    if let Err(e) = sd.read_block(0, &mut sector0) {
        crate::kprintln!("durable-media: error (mbr read {e:?})");
        return None;
    }
    let lba = match mbr::parse(&sector0) {
        Ok(entries) => match mbr::find_store_partition(&entries) {
            Some((lba, _sectors)) => lba,
            None => {
                crate::kprintln!("durable-media: no-partition (no 0x7f entry)");
                return None;
            }
        },
        Err(e) => {
            crate::kprintln!("durable-media: no-partition ({e:?})");
            return None;
        }
    };

    // Load runs BEFORE any put, so the counter read here is evidence of the
    // previous boot, not of this one.
    let mut loaded = [0u8; kernel_core::durable::REGION_SIZE];
    let winner = match sd.media_load(lba, &mut loaded) {
        Ok(w) => w,
        Err(e) => {
            crate::kprintln!("durable-media: error (load {e:?})");
            return None;
        }
    };
    if winner.is_some() {
        crate::durable::restore(&loaded);
    }

    let mut out = [0u8; 4];
    let prev = match crate::durable::get(b"boot", &mut out) {
        Ok(4) => u32::from_le_bytes(out),
        _ => 0,
    };
    let boot = prev + 1;
    match winner {
        Some((slot, seq)) => crate::kprintln!(
            "durable-media: boot={boot} from=Previous part=0x7f slot={slot:?} seq={seq} host={host}"
        ),
        None => crate::kprintln!(
            "durable-media: boot={boot} from=Fresh part=0x7f slot=- seq=0 host={host}"
        ),
    }
    if let Err(e) = crate::durable::put(b"boot", &boot.to_le_bytes()) {
        crate::kprintln!("durable-media: error (counter put {e:?})");
        return None;
    }

    Some(Witness { sd, lba, winner })
}

impl Witness {
    /// Snapshot the region and write the opposite slot (header last = commit),
    /// then read it back (ADR-0066).
    pub fn commit(&self) {
        let seq = self.winner.map(|(_, s)| s).unwrap_or(0);
        let snap = crate::durable::snapshot();
        match self
            .sd
            .media_flush(self.lba, self.winner.map(|(s, _)| s), seq, &snap)
        {
            Ok((slot, new_seq)) => {
                crate::kprintln!("durable-media: flushed slot={slot:?} seq={new_seq}");
                match self.sd.media_verify(self.lba, slot, new_seq, &snap) {
                    Ok(true) => crate::kprintln!("durable-media: verified"),
                    Ok(false) => crate::kprintln!("durable-media: error (verify mismatch)"),
                    Err(e) => crate::kprintln!("durable-media: error (verify {e:?})"),
                }
            }
            Err(e) => crate::kprintln!("durable-media: error (flush {e:?})"),
        }
    }
}
