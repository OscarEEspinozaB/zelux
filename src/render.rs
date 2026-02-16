use crate::terminal::{self, ColorMode};

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    Default,
    Ansi(u8),
    Color256(u8),
    Rgb(u8, u8, u8),
}

// ---------------------------------------------------------------------------
// Cell
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Screen
// ---------------------------------------------------------------------------

pub struct Screen {
    width: usize,
    height: usize,
    cells: Vec<Vec<Cell>>,
    prev_cells: Vec<Vec<Cell>>,
}

impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        let cells = make_grid(width, height);
        let prev_cells = Vec::new(); // empty → forces full draw on first flush
        Self {
            width,
            height,
            cells,
            prev_cells,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    // -- Building frames ---------------------------------------------------

    pub fn clear(&mut self) {
        for row in &mut self.cells {
            for cell in row.iter_mut() {
                *cell = Cell::default();
            }
        }
    }

    pub fn put_cell(&mut self, row: usize, col: usize, cell: Cell) {
        if row < self.height && col < self.width {
            self.cells[row][col] = cell;
        }
    }

    pub fn put_char(&mut self, row: usize, col: usize, ch: char, fg: Color, bg: Color, bold: bool) {
        self.put_cell(row, col, Cell { ch, fg, bg, bold });
    }

    pub fn put_str(
        &mut self,
        row: usize,
        col: usize,
        text: &str,
        fg: Color,
        bg: Color,
        bold: bool,
    ) {
        if row >= self.height {
            return;
        }
        let mut c = col;
        for ch in text.chars() {
            if c >= self.width {
                break;
            }
            self.cells[row][c] = Cell { ch, fg, bg, bold };
            c += 1;
        }
    }

    // -- Rendering ---------------------------------------------------------

    pub fn flush(&mut self, color_mode: &ColorMode) {
        let buf = self.build_diff_output(color_mode);
        if !buf.is_empty() {
            terminal::hide_cursor();
            terminal::write_all(&buf);
            terminal::show_cursor();
            terminal::flush();
        }
        // Swap: prev = current, then clear current for next frame
        self.prev_cells = self.cells.clone();
        self.clear();
    }

    // -- Resize ------------------------------------------------------------

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.cells = make_grid(width, height);
        self.prev_cells = Vec::new(); // force full redraw
    }

    // -- Internal ----------------------------------------------------------

    fn build_diff_output(&self, color_mode: &ColorMode) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4096);
        let mut cur_fg = Color::Default;
        let mut cur_bg = Color::Default;
        let mut cur_bold = false;
        let full_redraw = self.prev_cells.is_empty()
            || self.prev_cells.len() != self.height
            || (self.height > 0 && self.prev_cells[0].len() != self.width);

        for row in 0..self.height {
            for col in 0..self.width {
                let cell = &self.cells[row][col];
                let changed = if full_redraw {
                    true
                } else {
                    &self.prev_cells[row][col] != cell
                };
                if !changed {
                    continue;
                }

                // Position cursor (1-based)
                write_cursor_pos(&mut buf, row, col);

                // Apply style changes
                if cell.bold != cur_bold {
                    if cell.bold {
                        buf.extend_from_slice(b"\x1b[1m");
                    } else {
                        buf.extend_from_slice(b"\x1b[22m");
                    }
                    cur_bold = cell.bold;
                }
                if cell.fg != cur_fg {
                    write_fg_color(&mut buf, cell.fg, color_mode);
                    cur_fg = cell.fg;
                }
                if cell.bg != cur_bg {
                    write_bg_color(&mut buf, cell.bg, color_mode);
                    cur_bg = cell.bg;
                }

                // Write character
                write_char(&mut buf, cell.ch);
            }
        }

        // Reset attributes if we emitted anything
        if !buf.is_empty() {
            buf.extend_from_slice(b"\x1b[0m");
        }

        buf
    }
}

// ---------------------------------------------------------------------------
// Grid helper
// ---------------------------------------------------------------------------

fn make_grid(width: usize, height: usize) -> Vec<Vec<Cell>> {
    (0..height)
        .map(|_| (0..width).map(|_| Cell::default()).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// ANSI output helpers
// ---------------------------------------------------------------------------

fn write_cursor_pos(buf: &mut Vec<u8>, row: usize, col: usize) {
    // CSI row;col H  (1-based)
    buf.extend_from_slice(b"\x1b[");
    write_usize(buf, row + 1);
    buf.push(b';');
    write_usize(buf, col + 1);
    buf.push(b'H');
}

fn write_usize(buf: &mut Vec<u8>, n: usize) {
    if n == 0 {
        buf.push(b'0');
        return;
    }
    let start = buf.len();
    let mut v = n;
    while v > 0 {
        buf.push(b'0' + (v % 10) as u8);
        v /= 10;
    }
    buf[start..].reverse();
}

fn write_char(buf: &mut Vec<u8>, ch: char) {
    let mut tmp = [0u8; 4];
    buf.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
}

fn write_fg_color(buf: &mut Vec<u8>, color: Color, mode: &ColorMode) {
    match effective_color(color, mode) {
        Color::Default => buf.extend_from_slice(b"\x1b[39m"),
        Color::Ansi(n) => {
            let code = if n < 8 { 30 + n } else { 90 + n - 8 };
            buf.extend_from_slice(b"\x1b[");
            write_usize(buf, code as usize);
            buf.push(b'm');
        }
        Color::Color256(n) => {
            buf.extend_from_slice(b"\x1b[38;5;");
            write_usize(buf, n as usize);
            buf.push(b'm');
        }
        Color::Rgb(r, g, b) => {
            buf.extend_from_slice(b"\x1b[38;2;");
            write_usize(buf, r as usize);
            buf.push(b';');
            write_usize(buf, g as usize);
            buf.push(b';');
            write_usize(buf, b as usize);
            buf.push(b'm');
        }
    }
}

fn write_bg_color(buf: &mut Vec<u8>, color: Color, mode: &ColorMode) {
    match effective_color(color, mode) {
        Color::Default => buf.extend_from_slice(b"\x1b[49m"),
        Color::Ansi(n) => {
            let code = if n < 8 { 40 + n } else { 100 + n - 8 };
            buf.extend_from_slice(b"\x1b[");
            write_usize(buf, code as usize);
            buf.push(b'm');
        }
        Color::Color256(n) => {
            buf.extend_from_slice(b"\x1b[48;5;");
            write_usize(buf, n as usize);
            buf.push(b'm');
        }
        Color::Rgb(r, g, b) => {
            buf.extend_from_slice(b"\x1b[48;2;");
            write_usize(buf, r as usize);
            buf.push(b';');
            write_usize(buf, g as usize);
            buf.push(b';');
            write_usize(buf, b as usize);
            buf.push(b'm');
        }
    }
}

// ---------------------------------------------------------------------------
// Color downgrade
// ---------------------------------------------------------------------------

fn effective_color(color: Color, mode: &ColorMode) -> Color {
    match (color, mode) {
        (Color::Rgb(r, g, b), ColorMode::Color256) => Color::Color256(rgb_to_ansi256(r, g, b)),
        (Color::Rgb(r, g, b), ColorMode::Color16) => {
            Color::Ansi(ansi256_to_ansi16(rgb_to_ansi256(r, g, b)))
        }
        (Color::Color256(n), ColorMode::Color16) => Color::Ansi(ansi256_to_ansi16(n)),
        _ => color,
    }
}

/// Convert an RGB color to the nearest xterm-256 palette index.
///
/// The xterm-256 palette is:
///   0-7:     standard ANSI colors
///   8-15:    bright ANSI colors
///   16-231:  6x6x6 color cube
///   232-255: 24-step grayscale ramp
pub fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    // Check grayscale first
    if r == g && g == b {
        if r < 8 {
            return 16; // closest cube entry (black)
        }
        if r > 248 {
            return 231; // closest cube entry (white)
        }
        // Map to grayscale ramp 232-255 (values 8, 18, 28, ..., 238)
        return (((r as u16 - 8) * 24 / 240) as u8) + 232;
    }

    // Map to 6x6x6 color cube (indices 16-231)
    let ri = color_cube_index(r);
    let gi = color_cube_index(g);
    let bi = color_cube_index(b);
    16 + 36 * ri + 6 * gi + bi
}

fn color_cube_index(v: u8) -> u8 {
    // The 6 cube values are 0, 95, 135, 175, 215, 255
    // Midpoints: 48, 115, 155, 195, 235
    if v < 48 {
        0
    } else if v < 115 {
        1
    } else if v < 155 {
        2
    } else if v < 195 {
        3
    } else if v < 235 {
        4
    } else {
        5
    }
}

/// Map a 256-color index to the nearest standard 16-color ANSI index (0-15).
pub fn ansi256_to_ansi16(n: u8) -> u8 {
    match n {
        0..=15 => n,
        // Color cube and grayscale: approximate via perceived brightness
        _ => {
            let (r, g, b) = ansi256_to_rgb(n);
            // Weighted luminance
            let luma = (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000;

            // Find the nearest basic color
            // Simple mapping: use the 8 basic hues + bright variants
            let ri = if r > 128 { 1u8 } else { 0 };
            let gi = if g > 128 { 1u8 } else { 0 };
            let bi = if b > 128 { 1u8 } else { 0 };
            let base = bi << 2 | gi << 1 | ri; // ANSI color order: BGR

            if luma > 170 {
                base + 8 // bright variant
            } else {
                base
            }
        }
    }
}

fn ansi256_to_rgb(n: u8) -> (u8, u8, u8) {
    static ANSI_BASIC: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];

    match n {
        0..=15 => ANSI_BASIC[n as usize],
        16..=231 => {
            let idx = n - 16;
            let ri = idx / 36;
            let gi = (idx % 36) / 6;
            let bi = idx % 6;
            let cube = |i: u8| -> u8 { if i == 0 { 0 } else { 55 + 40 * i } };
            (cube(ri), cube(gi), cube(bi))
        }
        _ => {
            // Grayscale ramp 232-255: 8, 18, 28, ..., 238
            let v = 8 + 10 * (n - 232);
            (v, v, v)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_screen_dimensions() {
        let s = Screen::new(80, 24);
        assert_eq!(s.width(), 80);
        assert_eq!(s.height(), 24);
        assert_eq!(s.cells.len(), 24);
        assert_eq!(s.cells[0].len(), 80);
    }

    #[test]
    fn new_screen_default_cells() {
        let s = Screen::new(3, 2);
        for row in &s.cells {
            for cell in row {
                assert_eq!(*cell, Cell::default());
            }
        }
    }

    #[test]
    fn put_char_populates_cell() {
        let mut s = Screen::new(10, 5);
        s.put_char(2, 3, 'A', Color::Rgb(255, 0, 0), Color::Default, true);
        assert_eq!(s.cells[2][3].ch, 'A');
        assert_eq!(s.cells[2][3].fg, Color::Rgb(255, 0, 0));
        assert_eq!(s.cells[2][3].bold, true);
    }

    #[test]
    fn put_char_out_of_bounds() {
        let mut s = Screen::new(5, 5);
        // Should not panic
        s.put_char(10, 10, 'X', Color::Default, Color::Default, false);
    }

    #[test]
    fn put_str_populates_cells() {
        let mut s = Screen::new(10, 5);
        s.put_str(0, 0, "Hi!", Color::Default, Color::Default, false);
        assert_eq!(s.cells[0][0].ch, 'H');
        assert_eq!(s.cells[0][1].ch, 'i');
        assert_eq!(s.cells[0][2].ch, '!');
        assert_eq!(s.cells[0][3].ch, ' '); // untouched
    }

    #[test]
    fn put_str_truncates_at_edge() {
        let mut s = Screen::new(5, 1);
        s.put_str(0, 3, "Hello", Color::Default, Color::Default, false);
        assert_eq!(s.cells[0][3].ch, 'H');
        assert_eq!(s.cells[0][4].ch, 'e');
        // "llo" should be truncated
    }

    #[test]
    fn put_str_row_out_of_bounds() {
        let mut s = Screen::new(5, 2);
        s.put_str(5, 0, "nope", Color::Default, Color::Default, false);
        // Should not panic
    }

    #[test]
    fn clear_resets_cells() {
        let mut s = Screen::new(5, 3);
        s.put_char(1, 2, 'Z', Color::Ansi(1), Color::Ansi(2), true);
        s.clear();
        assert_eq!(s.cells[1][2], Cell::default());
    }

    #[test]
    fn resize_changes_dimensions() {
        let mut s = Screen::new(10, 5);
        s.resize(20, 10);
        assert_eq!(s.width(), 20);
        assert_eq!(s.height(), 10);
        assert_eq!(s.cells.len(), 10);
        assert_eq!(s.cells[0].len(), 20);
    }

    #[test]
    fn unchanged_screen_empty_diff() {
        let mut s = Screen::new(5, 3);
        // First flush: full draw (prev_cells empty)
        let first = s.build_diff_output(&ColorMode::TrueColor);
        assert!(!first.is_empty());

        // Copy current into prev (simulate flush without actual terminal I/O)
        s.prev_cells = s.cells.clone();
        s.clear();

        // Second flush with identical content: no diff
        let second = s.build_diff_output(&ColorMode::TrueColor);
        assert!(second.is_empty());
    }

    #[test]
    fn rgb_to_ansi256_black() {
        assert_eq!(rgb_to_ansi256(0, 0, 0), 16);
    }

    #[test]
    fn rgb_to_ansi256_white() {
        assert_eq!(rgb_to_ansi256(255, 255, 255), 231);
    }

    #[test]
    fn rgb_to_ansi256_gray() {
        let idx = rgb_to_ansi256(128, 128, 128);
        assert!((232..=255).contains(&idx));
    }

    #[test]
    fn rgb_to_ansi256_red() {
        let idx = rgb_to_ansi256(255, 0, 0);
        // Should map to the red region of the color cube
        // 255 → cube index 5, 0 → 0, 0 → 0 → 16 + 5*36 = 196
        assert_eq!(idx, 196);
    }

    #[test]
    fn ansi256_to_ansi16_passthrough() {
        for i in 0..=15 {
            assert_eq!(ansi256_to_ansi16(i), i);
        }
    }

    #[test]
    fn ansi256_to_ansi16_cube_color() {
        let n = ansi256_to_ansi16(196); // bright red
        // Should map to red (1) or bright red (9)
        assert!(n == 1 || n == 9);
    }

    #[test]
    fn cell_default_equality() {
        let a = Cell::default();
        let b = Cell {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
        };
        assert_eq!(a, b);
    }

    #[test]
    fn color_downgrade_rgb_to_256() {
        let c = effective_color(Color::Rgb(255, 0, 0), &ColorMode::Color256);
        assert_eq!(c, Color::Color256(196));
    }

    #[test]
    fn color_downgrade_rgb_to_16() {
        let c = effective_color(Color::Rgb(255, 0, 0), &ColorMode::Color16);
        if let Color::Ansi(n) = c {
            assert!(n <= 15);
        } else {
            panic!("Expected Ansi color");
        }
    }

    #[test]
    fn color_no_downgrade_in_truecolor() {
        let c = effective_color(Color::Rgb(42, 100, 200), &ColorMode::TrueColor);
        assert_eq!(c, Color::Rgb(42, 100, 200));
    }

    #[test]
    fn write_usize_zero() {
        let mut buf = Vec::new();
        write_usize(&mut buf, 0);
        assert_eq!(buf, b"0");
    }

    #[test]
    fn write_usize_multidigit() {
        let mut buf = Vec::new();
        write_usize(&mut buf, 123);
        assert_eq!(buf, b"123");
    }
}
