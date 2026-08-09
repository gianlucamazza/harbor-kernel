//! SDHCI host (BCM2711 EMMC2) — polled, single-block PIO (ADR-0066).
//!
//! Board-agnostic: the caller supplies the MMIO base. Register encodings
//! live in [`kernel_core::sdhci`], the card-init policy in
//! [`kernel_core::sdcard`], the on-media store layout in
//! [`kernel_core::durable_media`]; this module owns reset, clock bring-up,
//! command/data sequencing and the bounded polls between them.
//!
//! Deliberately narrow (the ADR's non-goals): 1-bit bus, ≤25 MHz, CMD17 /
//! CMD24 only, no DMA — every data byte moves through the 32-bit buffer
//! port under a spin budget.

use kernel_core::durable::REGION_SIZE;
use kernel_core::durable_media::{self as media, SECTOR_SIZE, Slot};
use kernel_core::poll;
use kernel_core::sdcard::{self, InitError, InitState};
use kernel_core::sdhci::{self as regs, DataDir};

use crate::arch::mmio::Mmio;
use crate::arch::probe;

/// Spin budgets. Each spin is an MMIO read (~100 ns class), so these are
/// generous fractions of a second — sized for silicon, not for QEMU's
/// permissive model, and bounded so a dead slot degrades boot instead of
/// hanging it.
const RESET_SPIN_LIMIT: u32 = 1_000_000;
const CLOCK_SPIN_LIMIT: u32 = 1_000_000;
const CMD_SPIN_LIMIT: u32 = 2_000_000;
const DATA_SPIN_LIMIT: u32 = 10_000_000;

/// Assumed base clock for the divided-clock arithmetic. Conservative by
/// construction (ADR-0066): if the real EMMC2 base is lower, every derived
/// clock is lower still — never out of spec, only slower.
const ASSUMED_BASE_CLOCK_HZ: u32 = 200_000_000;
const INIT_CLOCK_HZ: u32 = 400_000;
const DATA_CLOCK_HZ: u32 = 25_000_000;

/// Words per sector through the PIO buffer port.
const WORDS_PER_SECTOR: usize = SECTOR_SIZE / 4;

/// Why the SD path could not serve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdError {
    /// No controller answered at the MMIO window (external abort on probe).
    NotPresent,
    /// CMD8 went unanswered: empty slot or pre-v2 card — indistinguishable
    /// at this level, and reported as exactly that.
    NoCard,
    /// SDSC card, or a CMD8 echo mismatch — outside this slice's scope.
    Unsupported,
    /// A reset, clock, command or data wait exhausted its spin budget.
    Timeout,
    /// The host reported a CRC or transport error mid-command.
    Transport,
}

/// An initialised card behind an SDHCI host, ready for single-block I/O.
pub struct Sdhci {
    mmio: Mmio,
}

impl Sdhci {
    /// Probe, reset the host, bring up the clock, walk the card init.
    ///
    /// # Safety
    ///
    /// `mmio` must address an SDHCI register window exclusive to this
    /// driver for the duration of use, Device-mapped when the block exists.
    pub unsafe fn init(mmio: Mmio) -> Result<Self, SdError> {
        // Presence: one recoverable write into the window (rng200 pattern).
        // ARG2 is side-effect-free to write.
        // SAFETY: Device window; probe recovers an external abort.
        if unsafe { probe::try_write32(mmio.base() + regs::ARG2, 0) }.is_err() {
            return Err(SdError::NotPresent);
        }

        let host = Self { mmio };
        host.reset_host()?;
        host.set_clock(INIT_CLOCK_HZ)?;
        host.init_card()?;
        host.set_clock(DATA_CLOCK_HZ)?;
        Ok(host)
    }

    fn reset_host(&self) -> Result<(), SdError> {
        self.mmio.write32(regs::CONTROL1, regs::C1_SRST_HC);
        let m = self.mmio;
        if !poll::until(RESET_SPIN_LIMIT, || {
            m.read32(regs::CONTROL1) & regs::C1_SRST_HC == 0
        }) {
            return Err(SdError::Timeout);
        }
        // No card-interrupt routing in this slice: mask everything into the
        // status register only (polled), nothing toward the GIC.
        self.mmio.write32(regs::IRPT_EN, 0);
        self.mmio.write32(regs::IRPT_MASK, 0xFFFF_FFFF);
        // SD bus power at 3.3 V — commands do not leave the host without it.
        self.mmio
            .write32(regs::CONTROL0, regs::C0_VOLTAGE_3V3 | regs::C0_BUS_POWER);
        Ok(())
    }

    /// Program the divided clock: internal clock on, wait stable, enable.
    fn set_clock(&self, target_hz: u32) -> Result<(), SdError> {
        let div = regs::divider_for(ASSUMED_BASE_CLOCK_HZ, target_hz);
        self.mmio.write32(
            regs::CONTROL1,
            regs::C1_TOUNIT_MAX | regs::c1_clock_bits(div) | regs::C1_CLK_INTLEN,
        );
        let m = self.mmio;
        if !poll::until(CLOCK_SPIN_LIMIT, || {
            m.read32(regs::CONTROL1) & regs::C1_CLK_STABLE != 0
        }) {
            return Err(SdError::Timeout);
        }
        self.mmio
            .write32(regs::CONTROL1, m.read32(regs::CONTROL1) | regs::C1_CLK_EN);
        Ok(())
    }

    /// Walk [`kernel_core::sdcard`]'s machine: it decides, this issues.
    fn init_card(&self) -> Result<(), SdError> {
        let mut state = InitState::GoIdle;
        while let Some(req) = sdcard::request(state) {
            let resp0 = match self.issue(req.index, req.arg, req.resp, DataDir::None, req.checks) {
                Ok(r) => r,
                Err(SdError::Timeout) => {
                    return Err(match sdcard::on_timeout(state) {
                        InitError::NoResponse => SdError::NoCard,
                        InitError::Unsupported => SdError::Unsupported,
                        _ => SdError::Timeout,
                    });
                }
                Err(e) => return Err(e),
            };
            state = match sdcard::advance(state, resp0) {
                Ok(next) => next,
                Err(InitError::Unsupported | InitError::BadIfCond) => {
                    return Err(SdError::Unsupported);
                }
                Err(_) => return Err(SdError::Timeout),
            };
        }
        Ok(())
    }

    /// Issue one command, poll to completion, return `RESP0`.
    fn issue(
        &self,
        index: u8,
        arg: u32,
        resp: regs::RespType,
        dir: DataDir,
        checks: bool,
    ) -> Result<u32, SdError> {
        let m = self.mmio;
        if !poll::until(CMD_SPIN_LIMIT, || {
            m.read32(regs::STATUS) & (regs::STATUS_CMD_INHIBIT | regs::STATUS_DAT_INHIBIT) == 0
        }) {
            return Err(SdError::Timeout);
        }
        // Clear stale status (W1C) before arming a new command.
        self.mmio.write32(regs::INTERRUPT, 0xFFFF_FFFF);
        self.mmio.write32(regs::ARG1, arg);
        let tm = if checks {
            regs::cmdtm(index, resp, dir)
        } else {
            regs::cmdtm_no_checks(index, resp, dir)
        };
        self.mmio.write32(regs::CMDTM, tm);

        if !poll::until(CMD_SPIN_LIMIT, || {
            m.read32(regs::INTERRUPT) & (regs::INT_CMD_DONE | regs::INT_ERROR_MASK) != 0
        }) {
            return Err(SdError::Timeout);
        }
        let int = self.mmio.read32(regs::INTERRUPT);
        if int & regs::INT_ERROR_MASK != 0 {
            self.mmio.write32(regs::INTERRUPT, 0xFFFF_FFFF);
            return if regs::is_timeout_only(int) {
                Err(SdError::Timeout)
            } else {
                Err(SdError::Transport)
            };
        }
        Ok(self.mmio.read32(regs::RESP0))
    }

    /// Wait for a buffer-ready bit, honouring the error summary.
    fn wait_int(&self, bit: u32) -> Result<(), SdError> {
        let m = self.mmio;
        if !poll::until(DATA_SPIN_LIMIT, || {
            m.read32(regs::INTERRUPT) & (bit | regs::INT_ERROR_MASK) != 0
        }) {
            return Err(SdError::Timeout);
        }
        let int = self.mmio.read32(regs::INTERRUPT);
        if int & regs::INT_ERROR_MASK != 0 {
            self.mmio.write32(regs::INTERRUPT, 0xFFFF_FFFF);
            return Err(SdError::Transport);
        }
        // Consume only the awaited bit; DATA_DONE stays for its own wait.
        self.mmio.write32(regs::INTERRUPT, bit);
        Ok(())
    }

    /// Read one 512-byte sector (CMD17, PIO).
    pub fn read_block(&self, lba: u32, out: &mut [u8; SECTOR_SIZE]) -> Result<(), SdError> {
        self.mmio
            .write32(regs::BLKSIZECNT, (1 << 16) | regs::SECTOR_SIZE);
        self.issue(17, lba, regs::RespType::Short, DataDir::Read, true)?;
        self.wait_int(regs::INT_READ_RDY)?;
        for chunk in 0..WORDS_PER_SECTOR {
            let word = self.mmio.read32(regs::DATA);
            out[chunk * 4..chunk * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        self.wait_int(regs::INT_DATA_DONE)
    }

    /// Write one 512-byte sector (CMD24, PIO).
    pub fn write_block(&self, lba: u32, data: &[u8; SECTOR_SIZE]) -> Result<(), SdError> {
        self.mmio
            .write32(regs::BLKSIZECNT, (1 << 16) | regs::SECTOR_SIZE);
        self.issue(24, lba, regs::RespType::Short, DataDir::Write, true)?;
        self.wait_int(regs::INT_WRITE_RDY)?;
        for chunk in 0..WORDS_PER_SECTOR {
            let word = u32::from_le_bytes([
                data[chunk * 4],
                data[chunk * 4 + 1],
                data[chunk * 4 + 2],
                data[chunk * 4 + 3],
            ]);
            self.mmio.write32(regs::DATA, word);
        }
        self.wait_int(regs::INT_DATA_DONE)
    }

    /// Load the winning slot from the store partition at `part_lba` into
    /// `out`. `Ok(None)` = fresh media (no valid slot; `out` untouched).
    pub fn media_load(
        &self,
        part_lba: u32,
        out: &mut [u8; REGION_SIZE],
    ) -> Result<Option<(Slot, u64)>, SdError> {
        let mut header_a = [0u8; SECTOR_SIZE];
        let mut header_b = [0u8; SECTOR_SIZE];
        let mut payload_a = [0u8; REGION_SIZE];
        let mut payload_b = [0u8; REGION_SIZE];
        self.read_block(part_lba + Slot::A.header_sector(), &mut header_a)?;
        self.read_block(part_lba + Slot::B.header_sector(), &mut header_b)?;
        self.read_payload(part_lba, Slot::A, &mut payload_a)?;
        self.read_payload(part_lba, Slot::B, &mut payload_b)?;
        match media::pick_winner(&header_a, &payload_a, &header_b, &payload_b) {
            None => Ok(None),
            Some((slot, header)) => {
                out.copy_from_slice(match slot {
                    Slot::A => &payload_a,
                    Slot::B => &payload_b,
                });
                Ok(Some((slot, header.seq)))
            }
        }
    }

    /// Flush `payload` into the slot **opposite** the current winner:
    /// payload sectors first, header last (the commit point, ADR-0066).
    /// Returns the written slot and its sequence number.
    pub fn media_flush(
        &self,
        part_lba: u32,
        winner: Option<Slot>,
        seq: u64,
        payload: &[u8; REGION_SIZE],
    ) -> Result<(Slot, u64), SdError> {
        let slot = media::next_slot(winner);
        let next_seq = seq + 1;
        for s in 0..media::PAYLOAD_SECTORS {
            let mut sector = [0u8; SECTOR_SIZE];
            sector.copy_from_slice(&payload[s * SECTOR_SIZE..(s + 1) * SECTOR_SIZE]);
            self.write_block(part_lba + slot.payload_sector() + s as u32, &sector)?;
        }
        let header = media::encode_header(next_seq, payload);
        self.write_block(part_lba + slot.header_sector(), &header)?;
        Ok((slot, next_seq))
    }

    /// Re-read `slot` and confirm it commits exactly `payload` at `seq` —
    /// the read-back the flush oracle stands on.
    pub fn media_verify(
        &self,
        part_lba: u32,
        slot: Slot,
        seq: u64,
        payload: &[u8; REGION_SIZE],
    ) -> Result<bool, SdError> {
        let mut header = [0u8; SECTOR_SIZE];
        let mut read_back = [0u8; REGION_SIZE];
        self.read_block(part_lba + slot.header_sector(), &mut header)?;
        self.read_payload(part_lba, slot, &mut read_back)?;
        Ok(match media::validate(&header, &read_back) {
            Some(h) => h.seq == seq && read_back == *payload,
            None => false,
        })
    }

    fn read_payload(
        &self,
        part_lba: u32,
        slot: Slot,
        out: &mut [u8; REGION_SIZE],
    ) -> Result<(), SdError> {
        for s in 0..media::PAYLOAD_SECTORS {
            let mut sector = [0u8; SECTOR_SIZE];
            self.read_block(part_lba + slot.payload_sector() + s as u32, &mut sector)?;
            out[s * SECTOR_SIZE..(s + 1) * SECTOR_SIZE].copy_from_slice(&sector);
        }
        Ok(())
    }
}
