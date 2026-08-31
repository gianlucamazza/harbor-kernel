//! TFT status surface policy (ADR-0009).
//!
//! Structured slots only — not a serial log mirror. Painting is voluntary-path
//! only (boot, idle throttle, panic). Behind `debug-display`.

#![cfg(feature = "debug-display")]

use core::fmt::Write;

use kernel_core::display::Rgb565;
use kernel_core::font8x8::{GLYPH_H, GLYPH_W};
use kernel_core::telemetry::{Health, Snapshot};
use kernel_core::textgrid::TextGrid;
#[cfg(feature = "display-touch")]
use kernel_core::touch::{Calibration, CalibrationWizard};
#[cfg(feature = "display-touch")]
use kernel_core::ui::UiAction;
#[cfg(feature = "display-touch")]
use kernel_core::ui::calibration_target_contains;
use kernel_core::ui::{CALIBRATION_TARGETS, Page, SCREEN_HEIGHT, SCREEN_WIDTH, UiMode, UiState};

use crate::bsp::board::spi_display as display;
use crate::mm;
use crate::sync::Mutex;
use crate::time;

/// Dashboard columns / rows at 8×8, covering the full 480×320 panel.
pub const COLS: usize = 60;
pub const ROWS: usize = 36;

/// Update dynamic lines at most this often (timer ticks @ 10 Hz → 1 Hz).
const TICK_REFRESH_EVERY: u64 = 10;
#[cfg(feature = "display-touch")]
const CALIBRATION_CONFIRM_TIMEOUT: u64 = 300;

/// Grid + rate-limit state (architecture rule 7: no `static mut`).
///
/// Touched only on the voluntary path (boot, idle, panic with IRQs masked).
struct StatusState {
    grid: TextGrid<COLS, ROWS>,
    last_tick_paint: u64,
    ui: UiState,
    snapshot: Snapshot,
    pending_page: Option<Page>,
    #[cfg(feature = "display-touch")]
    calibration_deadline: u64,
    #[cfg(feature = "display-touch")]
    calibration: CalibrationWizard,
    #[cfg(feature = "display-touch")]
    pending_calibration: Option<Calibration>,
}

static STATUS: Mutex<StatusState> = Mutex::new(StatusState {
    grid: TextGrid::new(Rgb565::HARBOR),
    last_tick_paint: 0,
    ui: UiState::new(),
    snapshot: Snapshot::unknown(),
    pending_page: None,
    #[cfg(feature = "display-touch")]
    calibration_deadline: 0,
    #[cfg(feature = "display-touch")]
    calibration: CalibrationWizard::new(),
    #[cfg(feature = "display-touch")]
    pending_calibration: None,
});

/// Colours for the status surface.
const FG: Rgb565 = Rgb565::WHITE;
const BG: Rgb565 = Rgb565::HARBOR;
const FG_DIM: Rgb565 = Rgb565::from_rgb8(0xA0, 0xB0, 0xC0);
const FG_OK: Rgb565 = Rgb565::GREEN;
const FG_PANIC: Rgb565 = Rgb565::WHITE;
const BG_PANIC: Rgb565 = Rgb565::RED;
const PANEL: Rgb565 = Rgb565::from_rgb8(0x18, 0x32, 0x52);
const PANEL_DIM: Rgb565 = Rgb565::from_rgb8(0x10, 0x24, 0x3C);

/// Populate boot-time slots after panel + SPI are up, then paint dirty cells.
pub fn show_boot_after_display(cdiv: u32, bit_hz: u32, cntfrq_hz: u64) {
    with_status(|st| {
        st.snapshot.display.health = Health::Ready;
        st.snapshot.display.clock_divisor = cdiv;
        st.snapshot.display.bit_hz = bit_hz;
        st.ui.set_mode(UiMode::Ready);
        st.grid.clear(BG);
        refresh(st, cntfrq_hz);
        st.last_tick_paint = 0;
    });
}

/// Publish the result of the optional touch probe without claiming more than
/// the probe established.  The input agent owns subsequent counters.
#[cfg(feature = "display-touch")]
pub fn record_touch_health(health: Health) {
    with_status(|st| {
        st.snapshot.touch.health = health;
        if health != Health::Ready {
            st.ui.set_mode(UiMode::Degraded);
        }
    });
}

/// Rate-limited tick + heap lines (call from idle).
pub fn on_idle() {
    let ticks = time::ticks();
    #[cfg(feature = "display-touch")]
    if let Some((event, raw)) = crate::bsp::board::touch::poll_with_sample() {
        with_status(|st| {
            let (samples, errors, irq_low) = crate::bsp::board::touch::metrics();
            st.snapshot.touch.samples = samples;
            st.snapshot.touch.errors = errors;
            st.snapshot.touch.irq_low = irq_low;
            if errors != 0 {
                st.snapshot.touch.health = Health::Error;
                st.ui.set_mode(UiMode::Degraded);
            }
            let action = st.ui.handle_touch(event, ticks);
            match action {
                UiAction::ConfirmCalibration if st.pending_calibration.is_some() => {
                    let calibration = st.pending_calibration.unwrap();
                    if crate::calibration::commit(calibration) {
                        st.pending_calibration = None;
                        st.calibration = CalibrationWizard::new();
                        st.calibration_deadline = 0;
                        st.ui.set_mode(UiMode::Ready);
                        refresh(st, timer_frequency());
                    } else {
                        st.ui.set_mode(UiMode::Degraded);
                        refresh(st, timer_frequency());
                    }
                }
                UiAction::RetryCalibration => {
                    st.pending_calibration = None;
                    st.calibration = CalibrationWizard::new();
                    st.calibration_deadline = 0;
                    st.ui.set_mode(UiMode::Ready);
                    refresh(st, timer_frequency());
                }
                UiAction::CancelCalibration => {
                    st.pending_calibration = None;
                    st.calibration = CalibrationWizard::new();
                    st.calibration_deadline = 0;
                    st.ui.set_mode(UiMode::Ready);
                    refresh(st, timer_frequency());
                }
                _ if st.ui.page() == Page::Calibration
                    && event.kind == kernel_core::ui::TouchKind::Down
                    && calibration_target_contains(st.calibration.count(), event.x, event.y) =>
                {
                    if st.calibration.push(raw) && st.calibration.is_complete() {
                        st.pending_calibration = st.calibration.finish().ok();
                        if st.pending_calibration.is_some() {
                            st.ui.set_mode(UiMode::Confirm);
                            st.calibration_deadline =
                                ticks.saturating_add(CALIBRATION_CONFIRM_TIMEOUT);
                        } else {
                            st.calibration = CalibrationWizard::new();
                        }
                        refresh(st, timer_frequency());
                    }
                }
                UiAction::Navigate(_) => refresh(st, timer_frequency()),
                _ => {}
            }
        });
    }
    #[cfg(feature = "display-touch")]
    with_status(|st| {
        let (samples, errors, irq_low) = crate::bsp::board::touch::metrics();
        st.snapshot.touch.samples = samples;
        st.snapshot.touch.errors = errors;
        st.snapshot.touch.irq_low = irq_low;
        if errors != 0 {
            st.snapshot.touch.health = Health::Error;
            st.ui.set_mode(UiMode::Degraded);
        }
    });
    with_status(|st| {
        #[cfg(feature = "display-touch")]
        if st.ui.mode() == UiMode::Confirm && ticks >= st.calibration_deadline {
            st.pending_calibration = None;
            st.calibration = CalibrationWizard::new();
            st.calibration_deadline = 0;
            st.ui.set_mode(UiMode::Ready);
            refresh(st, timer_frequency());
        }
        if st.ui.on_tick(ticks) {
            refresh(st, timer_frequency());
        }
        if ticks.saturating_sub(st.last_tick_paint) < TICK_REFRESH_EVERY {
            return;
        }
        st.last_tick_paint = ticks;

        let mut buf = [0u8; COLS];
        let heap = mm::heap_remaining();
        let n = write_line(&mut buf, format_args!("ticks={ticks}  heap={heap}"));
        st.grid.set_line(5, &buf[..n], FG, BG);
        st.pending_page = Some(st.ui.page());
    });
}

fn timer_frequency() -> u64 {
    crate::arch::timer::frequency_hz()
}

fn refresh(st: &mut StatusState, cntfrq_hz: u64) {
    st.snapshot.display.frames = st.snapshot.display.frames.saturating_add(1);
    st.pending_page = Some(render_page(st, cntfrq_hz));
}

fn render_page(st: &mut StatusState, cntfrq_hz: u64) -> Page {
    let snapshot = st.snapshot;
    st.grid.clear(BG);
    match st.ui.page() {
        Page::Overview => {
            st.grid.set_line(0, b"HARBOR  OVERVIEW", FG, BG);
            st.grid.set_line(1, b"kernel           EL1 W^X", FG, BG);
            set_status_line(
                &mut st.grid,
                2,
                b"display SPI      ",
                snapshot.display.health,
                FG,
            );
            set_status_line(
                &mut st.grid,
                3,
                b"touch XPT2046    ",
                snapshot.touch.health,
                FG,
            );
            let mut buf = [0u8; COLS];
            let n = write_line(
                &mut buf,
                format_args!("SPI {} Hz  CNTFRQ {cntfrq_hz}", snapshot.display.bit_hz),
            );
            st.grid.set_line(4, &buf[..n], FG_DIM, BG);
            let n = write_line(
                &mut buf,
                format_args!(
                    "touch samples={} errors={}",
                    snapshot.touch.samples, snapshot.touch.errors
                ),
            );
            st.grid.set_line(5, &buf[..n], FG_DIM, BG);
        }
        Page::Agents => {
            st.grid.set_line(0, b"HARBOR  AGENTS", FG, BG);
            st.grid
                .set_line(1, b"agent facts    UNAVAILABLE", FG_DIM, BG);
            st.grid
                .set_line(2, b"screen agent   WINDOW CONTRACT", FG_DIM, BG);
            st.grid
                .set_line(3, b"authority      KERNEL CONTROLLED", FG_DIM, BG);
            st.grid
                .set_line(4, b"runtime        UNAVAILABLE", FG_DIM, BG);
        }
        Page::Resources => {
            st.grid.set_line(0, b"HARBOR  RESOURCES", FG, BG);
            st.grid.set_line(1, b"heap         SNAPSHOT", FG_DIM, BG);
            st.grid.set_line(2, b"frame pool   BOUNDED", FG_DIM, BG);
            st.grid.set_line(3, b"page tables  GUARDED", FG_DIM, BG);
            st.grid.set_line(4, b"authority    EXPLICIT", FG_DIM, BG);
            st.grid.set_line(5, b"scheduler    UNAVAILABLE", FG_DIM, BG);
        }
        Page::Network => {
            st.grid.set_line(0, b"HARBOR  NETWORK", FG, BG);
            set_status_line(
                &mut st.grid,
                1,
                b"GENET        ",
                snapshot.peripherals.genet,
                FG,
            );
            st.grid.set_line(2, b"RX/TX        CAPABILITY", FG_DIM, BG);
            set_status_line(
                &mut st.grid,
                3,
                b"storage      ",
                snapshot.peripherals.storage,
                FG,
            );
            st.grid.set_line(4, b"store        UNAVAILABLE", FG_DIM, BG);
            st.grid.set_line(5, b"transport    FAIL-CLOSED", FG_DIM, BG);
        }
        Page::Calibration => {
            st.grid.set_line(0, b"HARBOR  CALIBRATION", FG, BG);
            if st.ui.mode() == UiMode::Confirm {
                st.grid.set_line(1, b"CALIBRATION READY", FG_OK, BG);
                st.grid.set_line(2, b"USE SAVE OR CANCEL", FG, BG);
                st.grid.set_line(3, b"TOUCH BUTTONS BELOW", FG_DIM, BG);
            } else if st.ui.mode() == UiMode::Degraded {
                st.grid.set_line(1, b"CALIBRATION ERROR", BG_PANIC, BG);
                st.grid.set_line(2, b"USE RETRY OR CANCEL", FG, BG);
                st.grid.set_line(3, b"SAVE WAS NOT ACTIVATED", FG_DIM, BG);
            } else {
                let mut buf = [0u8; COLS];
                let n = write_line(
                    &mut buf,
                    format_args!("TAP TARGETS  {}/4", calibration_count(st)),
                );
                st.grid.set_line(1, &buf[..n], FG, BG);
                st.grid.set_line(2, b"TOUCH THE MARKED TARGET", FG_DIM, BG);
                st.grid.set_line(3, b"TARGETS ARE ORDERED 1-4", FG_DIM, BG);
            }
        }
        Page::Fault => {
            st.grid.set_line(0, b"HARBOR  FAULT MONITOR", FG, BG);
            let fault =
                snapshot.display.health == Health::Error || snapshot.touch.health == Health::Error;
            st.grid.set_line(
                1,
                if fault {
                    b"FAULT ACTIVE"
                } else {
                    b"NO ACTIVE FAULT"
                },
                if fault { BG_PANIC } else { FG_OK },
                BG,
            );
            st.grid.set_line(2, b"panic view reserved", FG_DIM, BG);
            st.grid.set_line(3, b"serial remains primary", FG_DIM, BG);
        }
    }
    let mut buf = [0u8; COLS];
    let n = write_line(
        &mut buf,
        format_args!(
            "cdiv={} frames={} flusherr={}",
            snapshot.display.clock_divisor, snapshot.display.frames, snapshot.display.flush_errors
        ),
    );
    st.grid.set_line(6, &buf[..n], FG_DIM, BG);
    st.grid
        .set_line(34, b"OVERV AGENT RESRC NET CALIB FAULT", FG_DIM, PANEL_DIM);
    st.ui.page()
}

fn set_status_line<const R: usize>(
    grid: &mut TextGrid<COLS, R>,
    row: usize,
    prefix: &[u8],
    health: Health,
    fg: Rgb565,
) {
    let mut buf = [b' '; COLS];
    let mut n = prefix.len().min(COLS);
    buf[..n].copy_from_slice(&prefix[..n]);
    let value = match health {
        Health::Ready => b"READY".as_slice(),
        Health::Unavailable => b"UNAVAILABLE".as_slice(),
        Health::Error => b"ERROR".as_slice(),
        Health::Unknown => b"UNKNOWN".as_slice(),
    };
    let take = value.len().min(COLS.saturating_sub(n));
    buf[n..n + take].copy_from_slice(&value[..take]);
    n += take;
    grid.set_line(row, &buf[..n], fg, BG);
}

#[cfg(feature = "display-touch")]
fn calibration_count(st: &StatusState) -> usize {
    st.calibration.count()
}

#[cfg(not(feature = "display-touch"))]
const fn calibration_count(_st: &StatusState) -> usize {
    0
}

fn paint_chrome(page: Page, mode: UiMode, calibration_step: usize) -> bool {
    display::with_display(|disp| {
        disp.with_panel(|panel| {
            let mut ok = panel.fill_rect(0, 0, SCREEN_WIDTH - 1, 31, PANEL).is_ok();
            ok &= panel
                .fill_rect(
                    0,
                    SCREEN_HEIGHT - 40,
                    SCREEN_WIDTH - 1,
                    SCREEN_HEIGHT - 1,
                    PANEL_DIM,
                )
                .is_ok();
            let tab_width = SCREEN_WIDTH / kernel_core::ui::TAB_COUNT as u16;
            let tab_x = page.index() as u16 * tab_width;
            ok &= panel
                .fill_rect(
                    tab_x,
                    SCREEN_HEIGHT - 40,
                    tab_x + tab_width.saturating_sub(2),
                    SCREEN_HEIGHT - 38,
                    FG_OK,
                )
                .is_ok();
            for x in [0, 159, 319] {
                ok &= panel.fill_rect(x, 45, x + 1, 258, PANEL_DIM).is_ok();
            }
            if page == Page::Calibration {
                if mode == UiMode::Confirm {
                    ok &= panel.fill_rect(40, 216, 200, 264, PANEL_DIM).is_ok();
                    ok &= panel.fill_rect(280, 216, 440, 264, FG_OK).is_ok();
                } else if mode == UiMode::Degraded {
                    ok &= panel.fill_rect(40, 216, 200, 264, FG_OK).is_ok();
                    ok &= panel.fill_rect(280, 216, 440, 264, PANEL_DIM).is_ok();
                } else if calibration_step < CALIBRATION_TARGETS.len() {
                    let (x, y) = CALIBRATION_TARGETS[calibration_step];
                    ok &= panel
                        .fill_rect(
                            x.saturating_sub(12),
                            y.saturating_sub(2),
                            x + 12,
                            y + 2,
                            FG_OK,
                        )
                        .is_ok();
                    ok &= panel
                        .fill_rect(
                            x.saturating_sub(2),
                            y.saturating_sub(12),
                            x + 2,
                            y + 12,
                            FG_OK,
                        )
                        .is_ok();
                }
            }
            ok
        })
    })
    .unwrap_or(false)
}

/// Panic banner on the glass (IRQs already masked).
pub fn show_panic(msg: &str) {
    with_status(|st| {
        st.grid.clear(BG_PANIC);
        st.grid
            .set_line(0, b"*** KERNEL PANIC ***", FG_PANIC, BG_PANIC);
        let mut buf = [0u8; COLS];
        let bytes = msg.as_bytes();
        let take = bytes.len().min(COLS);
        buf[..take].copy_from_slice(&bytes[..take]);
        st.grid.set_line(2, &buf[..take], FG_PANIC, BG_PANIC);
        st.grid
            .set_line(4, b"serial has full diagnostic", FG_PANIC, BG_PANIC);
        st.grid.set_line(5, b"*** halt ***", FG_PANIC, BG_PANIC);
        // Defer panel I/O until the status lock is released, even on the
        // panic path. The caller already has IRQs masked, so the bounded
        // snapshot flush remains safe without extending the state critical
        // section across SPI.
        st.pending_page = Some(Page::Fault);
    });
}

fn with_status(f: impl FnOnce(&mut StatusState)) {
    STATUS.with(f);
    flush_pending_frame();
}

fn write_line(buf: &mut [u8], args: core::fmt::Arguments<'_>) -> usize {
    buf.fill(b' ');
    let mut w = SliceWriter { buf, pos: 0 };
    let _ = w.write_fmt(args);
    w.pos.min(buf.len())
}

struct SliceWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Write for SliceWriter<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.pos >= self.buf.len() {
                break;
            }
            self.buf[self.pos] = b;
            self.pos += 1;
        }
        Ok(())
    }
}

struct FlushReport {
    bytes: u64,
}

fn record_flush(st: &mut StatusState, result: Result<FlushReport, ()>) {
    match result {
        Ok(report) => {
            st.snapshot.display.flushes = st.snapshot.display.flushes.saturating_add(1);
            st.snapshot.display.bytes = st.snapshot.display.bytes.saturating_add(report.bytes);
        }
        Err(()) => {
            st.snapshot.display.flush_errors = st.snapshot.display.flush_errors.saturating_add(1);
        }
    }
}

fn flush_pending_frame() {
    let Some((page, mode, calibration_step, mut snapshot)) = STATUS.with(|st| {
        let page = st.pending_page.take()?;
        #[cfg(feature = "display-touch")]
        let calibration_step = st.calibration.count();
        #[cfg(not(feature = "display-touch"))]
        let calibration_step = 0;
        Some((page, st.ui.mode(), calibration_step, st.grid.clone()))
    }) else {
        return;
    };
    let painted = paint_chrome(page, mode, calibration_step);
    let flushed = flush_dirty(&mut snapshot);
    let ok = painted && flushed.is_ok();
    STATUS.with(|st| {
        if ok {
            st.grid.acknowledge_snapshot(&snapshot);
        } else {
            st.grid.mark_all_dirty();
            st.snapshot.display.health = Health::Error;
            st.ui.set_mode(UiMode::Degraded);
        }
        record_flush(st, flushed);
    });
}

fn flush_dirty(grid: &mut TextGrid<COLS, ROWS>) -> Result<FlushReport, ()> {
    display::with_display(|disp| {
        disp.with_panel(|panel| {
            const MAX_RUN: usize = 4;
            let cell_bytes = (GLYPH_W * GLYPH_H * 2) as usize;
            let mut raster = [0u8; MAX_RUN * (GLYPH_W * GLYPH_H * 2) as usize];
            let mut bytes_sent = 0u64;
            let result: Result<(), crate::bsp::board::spi_display::PanelErr> = grid
                .drain_dirty_runs(MAX_RUN, |row, col, cells| {
                    for (index, cell) in cells.iter().copied().enumerate() {
                        let start = index * cell_bytes;
                        TextGrid::<COLS, ROWS>::raster_cell(
                            cell,
                            &mut raster[start..start + cell_bytes],
                        );
                    }
                    let (x, y) = TextGrid::<COLS, ROWS>::cell_origin(col, row);
                    let width = GLYPH_W.saturating_mul(cells.len() as u16);
                    let bytes = cells.len() * cell_bytes;
                    panel.blit_rgb565(x, y, width, GLYPH_H, &raster[..bytes])?;
                    bytes_sent = bytes_sent.saturating_add(bytes as u64);
                    Ok(())
                });
            result
                .map(|_| FlushReport { bytes: bytes_sent })
                .map_err(|_| ())
        })
    })
    .unwrap_or(Err(()))
}
