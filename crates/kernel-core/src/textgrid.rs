//! Fixed text grid with dirty tracking for a status surface.
//!
//! Geometry is compile-time: `COLS`×`ROWS` cells of a monospace glyph.
//! Only dirty cells need repainting (ADR-0009).

use crate::display::Rgb565;
use crate::font8x8::{self, GLYPH_H, GLYPH_W};

/// One character cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: u8,
    pub fg: Rgb565,
    pub bg: Rgb565,
}

impl Cell {
    pub const fn blank(bg: Rgb565) -> Self {
        Self {
            ch: b' ',
            fg: Rgb565::WHITE,
            bg,
        }
    }
}

/// Pixel size of the grid.
pub const fn grid_pixel_size(cols: u16, rows: u16) -> (u16, u16) {
    (cols.saturating_mul(GLYPH_W), rows.saturating_mul(GLYPH_H))
}

/// Status grid: fixed columns and rows of 8×8 cells.
pub struct TextGrid<const COLS: usize, const ROWS: usize> {
    cells: [[Cell; COLS]; ROWS],
    /// Bit i of `dirty[row]` is set when `cells[row][i]` must be painted.
    dirty: [u64; ROWS],
}

impl<const COLS: usize, const ROWS: usize> TextGrid<COLS, ROWS> {
    /// Empty grid filled with `bg`.
    pub const fn new(bg: Rgb565) -> Self {
        assert!(COLS <= 64, "dirty mask is u64");
        let blank = Cell::blank(bg);
        Self {
            cells: [[blank; COLS]; ROWS],
            dirty: [0; ROWS],
        }
    }

    pub const fn cols(&self) -> usize {
        COLS
    }

    pub const fn rows(&self) -> usize {
        ROWS
    }

    /// Write a line of ASCII at `row`, padded/truncated to `COLS`.
    /// Marks changed cells dirty.
    pub fn set_line(&mut self, row: usize, text: &[u8], fg: Rgb565, bg: Rgb565) {
        if row >= ROWS {
            return;
        }
        for col in 0..COLS {
            let ch = text.get(col).copied().unwrap_or(b' ');
            let next = Cell { ch, fg, bg };
            if self.cells[row][col] != next {
                self.cells[row][col] = next;
                self.dirty[row] |= 1u64 << col;
            }
        }
    }

    /// Mark every cell dirty (e.g. after a full clear).
    pub fn mark_all_dirty(&mut self) {
        let mask = if COLS == 64 {
            u64::MAX
        } else {
            (1u64 << COLS) - 1
        };
        for d in &mut self.dirty {
            *d = mask;
        }
    }

    /// Clear all cells to space on `bg` and mark dirty.
    pub fn clear(&mut self, bg: Rgb565) {
        let blank = Cell::blank(bg);
        for row in 0..ROWS {
            for col in 0..COLS {
                self.cells[row][col] = blank;
            }
        }
        self.mark_all_dirty();
    }

    /// Iterate dirty cells as `(row, col, cell)` and clear their dirty bits.
    pub fn drain_dirty(&mut self, mut f: impl FnMut(usize, usize, Cell)) {
        for row in 0..ROWS {
            let mut bits = self.dirty[row];
            while bits != 0 {
                let col = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if col < COLS {
                    f(row, col, self.cells[row][col]);
                }
            }
            self.dirty[row] = 0;
        }
    }

    /// Pixel origin of cell `(col, row)`.
    pub const fn cell_origin(col: usize, row: usize) -> (u16, u16) {
        (
            (col as u16).saturating_mul(GLYPH_W),
            (row as u16).saturating_mul(GLYPH_H),
        )
    }

    /// Raster one cell into `out` as RGB565 big-endian pairs (length ≥ 128).
    pub fn raster_cell(cell: Cell, out: &mut [u8]) {
        debug_assert!(out.len() >= (GLYPH_W * GLYPH_H * 2) as usize);
        let g = font8x8::glyph(cell.ch);
        let fg = cell.fg.to_be_bytes();
        let bg = cell.bg.to_be_bytes();
        let mut i = 0;
        for bits in g.iter().take(8) {
            for col in 0..8 {
                let px = if font8x8::pixel_set(*bits, col) {
                    fg
                } else {
                    bg
                };
                out[i] = px[0];
                out[i + 1] = px[1];
                i += 2;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_line_dirties_changed_cells_only() {
        let mut g = TextGrid::<8, 2>::new(Rgb565::BLACK);
        g.set_line(0, b"Hi", Rgb565::WHITE, Rgb565::BLACK);
        let mut n = 0;
        g.drain_dirty(|_, _, _| n += 1);
        // Only 'H' and 'i' differ from blank spaces.
        assert_eq!(n, 2);
        g.set_line(0, b"Hi", Rgb565::WHITE, Rgb565::BLACK);
        let mut n2 = 0;
        g.drain_dirty(|_, _, _| n2 += 1);
        assert_eq!(n2, 0, "identical line is not dirty");
    }

    #[test]
    fn raster_space_is_background() {
        let mut buf = [0u8; 128];
        TextGrid::<1, 1>::raster_cell(Cell::blank(Rgb565::HARBOR), &mut buf);
        let hb = Rgb565::HARBOR.to_be_bytes();
        assert!(buf.chunks(2).all(|c| c == hb));
    }

    #[test]
    fn grid_pixel_size_matches_font() {
        assert_eq!(grid_pixel_size(60, 8), (480, 64));
    }
}
