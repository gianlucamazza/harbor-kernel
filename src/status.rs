//! TFT status surface policy (ADR-0009).
//!
//! Structured slots only — not a serial log mirror. Painting is voluntary-path
//! only (boot, idle throttle, panic). Behind `debug-display`.

#![cfg(feature = "debug-display")]

use core::fmt::Write;

use kernel_core::display::Rgb565;
use kernel_core::font8x8::{GLYPH_H, GLYPH_W};
use kernel_core::textgrid::TextGrid;

use crate::bsp::rpi4::display;
use crate::mm;
use crate::time;

/// Status columns / rows at 8×8 (fits 480×320 with room for margins).
pub const COLS: usize = 60;
pub const ROWS: usize = 8;

/// Update dynamic lines at most this often (timer ticks @ 10 Hz → 1 Hz).
const TICK_REFRESH_EVERY: u64 = 10;

static mut GRID: TextGrid<COLS, ROWS> = TextGrid::new(Rgb565::HARBOR);
static mut LAST_TICK_PAINT: u64 = 0;

/// Colours for the status surface.
const FG: Rgb565 = Rgb565::WHITE;
const BG: Rgb565 = Rgb565::HARBOR;
const FG_DIM: Rgb565 = Rgb565::from_rgb8(0xA0, 0xB0, 0xC0);
const FG_OK: Rgb565 = Rgb565::GREEN;
const FG_PANIC: Rgb565 = Rgb565::WHITE;
const BG_PANIC: Rgb565 = Rgb565::RED;

/// Populate boot-time slots after panel + SPI are up, then paint dirty cells.
pub fn show_boot_after_display(cdiv: u32, bit_hz: u32, cntfrq_hz: u64) {
    // SAFETY: single core; called from bootstrap before/around IRQ unmask.
    let grid = unsafe { &mut *core::ptr::addr_of_mut!(GRID) };
    grid.clear(BG);
    grid.set_line(0, b"Harbor  debug-display", FG, BG);
    grid.set_line(1, b"EL1  W^X  cooperative", FG_DIM, BG);

    let mut buf = [0u8; COLS];
    let n = write_line(&mut buf, format_args!("CNTFRQ={cntfrq_hz} Hz"));
    grid.set_line(2, &buf[..n], FG_DIM, BG);

    grid.set_line(3, b"rng200: see serial log", FG_OK, BG);

    let n = write_line(
        &mut buf,
        format_args!("SPI cdiv={cdiv}  {bit_hz} Hz"),
    );
    grid.set_line(4, &buf[..n], FG_DIM, BG);

    grid.set_line(5, b"ticks=--  heap=--", FG, BG);
    grid.set_line(6, b"", FG, BG);
    grid.set_line(7, b"UART primary  TFT status", FG_DIM, BG);

    flush_dirty(grid);
    unsafe {
        LAST_TICK_PAINT = 0;
    }
}

/// Rate-limited tick + heap lines (call from idle).
pub fn on_idle() {
    let ticks = time::ticks();
    let last = unsafe { LAST_TICK_PAINT };
    if ticks.saturating_sub(last) < TICK_REFRESH_EVERY {
        return;
    }
    unsafe {
        LAST_TICK_PAINT = ticks;
    }

    let grid = unsafe { &mut *core::ptr::addr_of_mut!(GRID) };
    let mut buf = [0u8; COLS];
    let heap = mm::heap_remaining();
    let n = write_line(
        &mut buf,
        format_args!("ticks={ticks}  heap={heap}"),
    );
    grid.set_line(5, &buf[..n], FG, BG);
    flush_dirty(grid);
}

/// Panic banner on the glass (IRQs already masked).
pub fn show_panic(msg: &str) {
    let grid = unsafe { &mut *core::ptr::addr_of_mut!(GRID) };
    grid.clear(BG_PANIC);
    grid.set_line(0, b"*** KERNEL PANIC ***", FG_PANIC, BG_PANIC);
    let mut buf = [0u8; COLS];
    let bytes = msg.as_bytes();
    let take = bytes.len().min(COLS);
    buf[..take].copy_from_slice(&bytes[..take]);
    grid.set_line(2, &buf[..take], FG_PANIC, BG_PANIC);
    grid.set_line(4, b"serial has full diagnostic", FG_PANIC, BG_PANIC);
    grid.set_line(5, b"*** halt ***", FG_PANIC, BG_PANIC);
    flush_dirty(grid);
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
