//! TFT status surface policy (ADR-0009).
//!
//! Structured slots only — not a serial log mirror. Painting is voluntary-path
//! only (boot, idle throttle, panic). Behind `debug-display`.

#![cfg(feature = "debug-display")]

use core::fmt::Write;

use kernel_core::display::Rgb565;
use kernel_core::font8x8::{GLYPH_H, GLYPH_W};
use kernel_core::textgrid::TextGrid;
use kernel_core::ui::{Page, SCREEN_HEIGHT, SCREEN_WIDTH, UiState};

use crate::bsp::board::spi_display as display;
use crate::mm;
use crate::sync::Mutex;
use crate::time;

/// Dashboard columns / rows at 8×8, covering the full 480×320 panel.
pub const COLS: usize = 60;
pub const ROWS: usize = 36;

/// Update dynamic lines at most this often (timer ticks @ 10 Hz → 1 Hz).
const TICK_REFRESH_EVERY: u64 = 10;

/// Grid + rate-limit state (architecture rule 7: no `static mut`).
///
/// Touched only on the voluntary path (boot, idle, panic with IRQs masked).
struct StatusState {
    grid: TextGrid<COLS, ROWS>,
    last_tick_paint: u64,
    ui: UiState,
}

static STATUS: Mutex<StatusState> = Mutex::new(StatusState {
    grid: TextGrid::new(Rgb565::HARBOR),
    last_tick_paint: 0,
    ui: UiState::new(),
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
        st.grid.clear(BG);
        render_page(st, cdiv, bit_hz, cntfrq_hz);
        flush_dirty(&mut st.grid);
        st.last_tick_paint = 0;
    });
}

/// Rate-limited tick + heap lines (call from idle).
pub fn on_idle() {
    let ticks = time::ticks();
    #[cfg(feature = "display-touch")]
    if let Some(event) = crate::bsp::board::touch::poll() {
        with_status(|st| {
            if st.ui.on_touch(event, ticks) {
                render_page(st, 64, 7_812_500, timer_frequency());
                flush_dirty(&mut st.grid);
            }
        });
    }
    with_status(|st| {
        if st.ui.on_tick(ticks) {
            render_page(st, 64, 7_812_500, timer_frequency());
            flush_dirty(&mut st.grid);
        }
        if ticks.saturating_sub(st.last_tick_paint) < TICK_REFRESH_EVERY {
            return;
        }
        st.last_tick_paint = ticks;

        let mut buf = [0u8; COLS];
        let heap = mm::heap_remaining();
        let n = write_line(&mut buf, format_args!("ticks={ticks}  heap={heap}"));
        st.grid.set_line(5, &buf[..n], FG, BG);
        flush_dirty(&mut st.grid);
    });
}

fn timer_frequency() -> u64 {
    crate::arch::timer::frequency_hz()
}

fn render_page(st: &mut StatusState, cdiv: u32, bit_hz: u32, cntfrq_hz: u64) {
    st.grid.clear(BG);
    match st.ui.page() {
        Page::Overview => {
            st.grid.set_line(0, b"HARBOR  OVERVIEW", FG, BG);
            st.grid.set_line(1, b"kernel healthy   EL1 W^X", FG_OK, BG);
            st.grid.set_line(2, b"display SPI      ONLINE", FG, BG);
            st.grid
                .set_line(3, b"touch XPT2046    OPTIONAL", FG_DIM, BG);
            let mut buf = [0u8; COLS];
            let n = write_line(
                &mut buf,
                format_args!("SPI {bit_hz} Hz  CNTFRQ {cntfrq_hz}"),
            );
            st.grid.set_line(4, &buf[..n], FG_DIM, BG);
            st.grid.set_line(5, b"tap a tab for details", FG, BG);
        }
        Page::Agents => {
            st.grid.set_line(0, b"HARBOR  AGENTS", FG, BG);
            st.grid.set_line(1, b"beacon       RUNNING", FG_OK, BG);
            st.grid.set_line(2, b"chirp        RUNNING", FG_OK, BG);
            st.grid.set_line(3, b"lookup       READY", FG_OK, BG);
            st.grid.set_line(4, b"entropy      WINDOW 0", FG_DIM, BG);
            st.grid.set_line(5, b"screen       WINDOW 1", FG_DIM, BG);
        }
        Page::Resources => {
            st.grid.set_line(0, b"HARBOR  RESOURCES", FG, BG);
            st.grid.set_line(1, b"heap         LIVE", FG_OK, BG);
            st.grid.set_line(2, b"frame pool   BOUNDED", FG_OK, BG);
            st.grid.set_line(3, b"page tables  GUARDED", FG_OK, BG);
            st.grid.set_line(4, b"authority    EXPLICIT", FG_DIM, BG);
            st.grid.set_line(5, b"scheduler    DUAL CORE", FG_DIM, BG);
        }
        Page::Network => {
            st.grid.set_line(0, b"HARBOR  NETWORK", FG, BG);
            st.grid.set_line(1, b"GENET        PROBED", FG_OK, BG);
            st.grid.set_line(2, b"RX/TX        CAPABILITY", FG_DIM, BG);
            st.grid.set_line(3, b"storage      SD READY", FG_OK, BG);
            st.grid.set_line(4, b"store        7 AGENTS", FG_DIM, BG);
            st.grid.set_line(5, b"transport    FAIL-CLOSED", FG_DIM, BG);
        }
        Page::Fault => {
            st.grid.set_line(0, b"HARBOR  FAULT MONITOR", FG, BG);
            st.grid.set_line(1, b"no active fault", FG_OK, BG);
            st.grid.set_line(2, b"panic view reserved", FG_DIM, BG);
            st.grid.set_line(3, b"serial remains primary", FG_DIM, BG);
        }
    }
    let mut buf = [0u8; COLS];
    let n = write_line(&mut buf, format_args!("ticks=--  heap=--  cdiv={cdiv}"));
    st.grid.set_line(6, &buf[..n], FG_DIM, BG);
    st.grid
        .set_line(34, b"OVERV AGENT RESRC NET   FAULT", FG_DIM, PANEL_DIM);
    paint_chrome(st.ui.page());
}

fn paint_chrome(page: Page) {
    display::with_display(|disp| {
        disp.with_panel(|panel| {
            let _ = panel.fill_rect(0, 0, SCREEN_WIDTH - 1, 31, PANEL);
            let _ = panel.fill_rect(
                0,
                SCREEN_HEIGHT - 40,
                SCREEN_WIDTH - 1,
                SCREEN_HEIGHT - 1,
                PANEL_DIM,
            );
            let tab_x = page.index() as u16 * (SCREEN_WIDTH / 5);
            let _ = panel.fill_rect(
                tab_x,
                SCREEN_HEIGHT - 40,
                tab_x + 94,
                SCREEN_HEIGHT - 38,
                FG_OK,
            );
            for x in [0, 159, 319] {
                let _ = panel.fill_rect(x, 45, x + 1, 258, PANEL_DIM);
            }
        });
    });
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
        flush_dirty(&mut st.grid);
    });
}

fn with_status(f: impl FnOnce(&mut StatusState)) {
    STATUS.with(f);
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

fn flush_dirty(grid: &mut TextGrid<COLS, ROWS>) {
    display::with_display(|disp| {
        disp.with_panel(|panel| {
            let mut raster = [0u8; (GLYPH_W * GLYPH_H * 2) as usize];
            grid.drain_dirty(|row, col, cell| {
                TextGrid::<COLS, ROWS>::raster_cell(cell, &mut raster);
                let (x, y) = TextGrid::<COLS, ROWS>::cell_origin(col, row);
                let _ = panel.blit_rgb565(x, y, GLYPH_W, GLYPH_H, &raster);
            });
        });
    });
}
