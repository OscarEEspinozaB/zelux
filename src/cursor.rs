use crate::buffer::Buffer;

pub struct Cursor {
    pub line: usize,
    pub col: usize,
    pub desired_col: usize,
}

impl Cursor {
    pub fn new() -> Cursor {
        Cursor {
            line: 0,
            col: 0,
            desired_col: 0,
        }
    }

    pub fn set_position(&mut self, line: usize, col: usize, buf: &Buffer) {
        self.line = line.min(buf.line_count().saturating_sub(1));
        let line_len = line_byte_len(buf, self.line);
        self.col = col.min(line_len);
        self.desired_col = self.col;
    }

    pub fn move_left(&mut self, buf: &Buffer) {
        if self.col > 0 {
            let line_text = buf.get_line(self.line).unwrap_or_default();
            self.col = prev_char_boundary(&line_text, self.col);
        } else if self.line > 0 {
            self.line -= 1;
            self.col = line_byte_len(buf, self.line);
        }
        self.desired_col = self.col;
    }

    pub fn move_right(&mut self, buf: &Buffer) {
        let line_len = line_byte_len(buf, self.line);
        if self.col < line_len {
            let line_text = buf.get_line(self.line).unwrap_or_default();
            self.col = next_char_boundary(&line_text, self.col);
        } else if self.line + 1 < buf.line_count() {
            self.line += 1;
            self.col = 0;
        }
        self.desired_col = self.col;
    }

    pub fn move_up(&mut self, buf: &Buffer) {
        if self.line > 0 {
            self.line -= 1;
            let line_len = line_byte_len(buf, self.line);
            self.col = self.desired_col.min(line_len);
        }
    }

    pub fn move_down(&mut self, buf: &Buffer) {
        if self.line + 1 < buf.line_count() {
            self.line += 1;
            let line_len = line_byte_len(buf, self.line);
            self.col = self.desired_col.min(line_len);
        }
    }

    pub fn move_word_left(&mut self, buf: &Buffer) {
        // If at start of line, wrap to end of previous line
        if self.col == 0 {
            if self.line > 0 {
                self.line -= 1;
                self.col = line_byte_len(buf, self.line);
            }
            self.desired_col = self.col;
            return;
        }

        let line_text = buf.get_line(self.line).unwrap_or_default();
        let bytes = line_text.as_bytes();
        let mut pos = self.col;

        // Skip non-word chars backwards
        while pos > 0 && !is_word_byte(bytes[pos - 1]) {
            pos -= 1;
        }
        // Skip word chars backwards
        while pos > 0 && is_word_byte(bytes[pos - 1]) {
            pos -= 1;
        }

        self.col = pos;
        self.desired_col = self.col;
    }

    pub fn move_word_right(&mut self, buf: &Buffer) {
        let line_len = line_byte_len(buf, self.line);

        // If at end of line, wrap to start of next line
        if self.col >= line_len {
            if self.line + 1 < buf.line_count() {
                self.line += 1;
                self.col = 0;
            }
            self.desired_col = self.col;
            return;
        }

        let line_text = buf.get_line(self.line).unwrap_or_default();
        let bytes = line_text.as_bytes();
        let len = bytes.len();
        let mut pos = self.col;

        // Skip word chars forward
        while pos < len && is_word_byte(bytes[pos]) {
            pos += 1;
        }
        // Skip non-word chars forward
        while pos < len && !is_word_byte(bytes[pos]) {
            pos += 1;
        }

        self.col = pos;
        self.desired_col = self.col;
    }

    pub fn move_home(&mut self, buf: &Buffer) {
        let line_text = buf.get_line(self.line).unwrap_or_default();
        let first_non_ws = line_text
            .bytes()
            .position(|b| b != b' ' && b != b'\t')
            .unwrap_or(0);

        if self.col > first_non_ws {
            self.col = first_non_ws;
        } else if self.col == first_non_ws && first_non_ws != 0 {
            self.col = 0;
        } else {
            self.col = first_non_ws;
        }
        self.desired_col = self.col;
    }

    pub fn move_end(&mut self, buf: &Buffer) {
        self.col = line_byte_len(buf, self.line);
        self.desired_col = self.col;
    }

    pub fn move_page_up(&mut self, buf: &Buffer, page_height: usize) {
        self.line = self.line.saturating_sub(page_height);
        let line_len = line_byte_len(buf, self.line);
        self.col = self.desired_col.min(line_len);
    }

    pub fn move_page_down(&mut self, buf: &Buffer, page_height: usize) {
        let max_line = buf.line_count().saturating_sub(1);
        self.line = (self.line + page_height).min(max_line);
        let line_len = line_byte_len(buf, self.line);
        self.col = self.desired_col.min(line_len);
    }

    pub fn move_to_start(&mut self) {
        self.line = 0;
        self.col = 0;
        self.desired_col = 0;
    }

    pub fn move_to_end(&mut self, buf: &Buffer) {
        self.line = buf.line_count().saturating_sub(1);
        self.col = line_byte_len(buf, self.line);
        self.desired_col = self.col;
    }

    pub fn byte_offset(&self, buf: &Buffer) -> usize {
        let line_start = buf.line_start(self.line).unwrap_or(0);
        line_start + self.col
    }

    pub fn clamp(&mut self, buf: &Buffer) {
        let max_line = buf.line_count().saturating_sub(1);
        if self.line > max_line {
            self.line = max_line;
        }
        let line_len = line_byte_len(buf, self.line);
        if self.col > line_len {
            self.col = line_len;
        }
    }
}

fn line_byte_len(buf: &Buffer, line: usize) -> usize {
    buf.get_line(line).map_or(0, |s| s.len())
}

fn prev_char_boundary(line: &str, byte_col: usize) -> usize {
    let bytes = line.as_bytes();
    let mut pos = byte_col;
    if pos == 0 {
        return 0;
    }
    pos -= 1;
    // Walk back over continuation bytes (10xxxxxx)
    while pos > 0 && bytes[pos] & 0xC0 == 0x80 {
        pos -= 1;
    }
    pos
}

fn next_char_boundary(line: &str, byte_col: usize) -> usize {
    let bytes = line.as_bytes();
    let len = bytes.len();
    if byte_col >= len {
        return len;
    }
    let mut pos = byte_col + 1;
    // Walk forward over continuation bytes (10xxxxxx)
    while pos < len && bytes[pos] & 0xC0 == 0x80 {
        pos += 1;
    }
    pos
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf_with(text: &str) -> Buffer {
        let mut buf = Buffer::new();
        buf.insert(0, text);
        buf
    }

    #[test]
    fn test_new_cursor() {
        let c = Cursor::new();
        assert_eq!(c.line, 0);
        assert_eq!(c.col, 0);
        assert_eq!(c.desired_col, 0);
    }

    #[test]
    fn test_move_right_single_line() {
        let buf = buf_with("abc");
        let mut c = Cursor::new();
        c.move_right(&buf);
        assert_eq!(c.col, 1);
        c.move_right(&buf);
        assert_eq!(c.col, 2);
        c.move_right(&buf);
        assert_eq!(c.col, 3);
        // At end of last line, stays
        c.move_right(&buf);
        assert_eq!(c.col, 3);
        assert_eq!(c.line, 0);
    }

    #[test]
    fn test_move_right_wraps() {
        let buf = buf_with("ab\ncd");
        let mut c = Cursor::new();
        c.move_right(&buf); // col 1
        c.move_right(&buf); // col 2 (end of "ab")
        c.move_right(&buf); // wraps to line 1, col 0
        assert_eq!(c.line, 1);
        assert_eq!(c.col, 0);
    }

    #[test]
    fn test_move_left_wraps() {
        let buf = buf_with("ab\ncd");
        let mut c = Cursor::new();
        c.line = 1;
        c.col = 0;
        c.move_left(&buf); // wraps to end of line 0
        assert_eq!(c.line, 0);
        assert_eq!(c.col, 2);
    }

    #[test]
    fn test_move_left_at_start() {
        let buf = buf_with("abc");
        let mut c = Cursor::new();
        c.move_left(&buf);
        assert_eq!(c.line, 0);
        assert_eq!(c.col, 0);
    }

    #[test]
    fn test_move_up_down_desired_col() {
        // Line 0: "long line here" (14 chars)
        // Line 1: "short" (5 chars)
        // Line 2: "another long line" (17 chars)
        let buf = buf_with("long line here\nshort\nanother long line");
        let mut c = Cursor::new();
        c.set_position(0, 10, &buf);
        assert_eq!(c.desired_col, 10);

        c.move_down(&buf);
        assert_eq!(c.line, 1);
        assert_eq!(c.col, 5); // clamped to "short" length
        assert_eq!(c.desired_col, 10); // preserved

        c.move_down(&buf);
        assert_eq!(c.line, 2);
        assert_eq!(c.col, 10); // restored from desired_col
    }

    #[test]
    fn test_move_up_at_top() {
        let buf = buf_with("abc\ndef");
        let mut c = Cursor::new();
        c.move_up(&buf);
        assert_eq!(c.line, 0);
        assert_eq!(c.col, 0);
    }

    #[test]
    fn test_move_down_at_bottom() {
        let buf = buf_with("abc\ndef");
        let mut c = Cursor::new();
        c.line = 1;
        c.col = 2;
        c.desired_col = 2;
        c.move_down(&buf);
        assert_eq!(c.line, 1);
        assert_eq!(c.col, 2);
    }

    #[test]
    fn test_move_word_left() {
        let buf = buf_with("hello world foo");
        let mut c = Cursor::new();
        c.col = 15; // end
        c.desired_col = 15;

        c.move_word_left(&buf);
        assert_eq!(c.col, 12); // start of "foo"

        c.move_word_left(&buf);
        assert_eq!(c.col, 6); // start of "world"

        c.move_word_left(&buf);
        assert_eq!(c.col, 0); // start of "hello"

        c.move_word_left(&buf);
        assert_eq!(c.col, 0); // stays at 0
    }

    #[test]
    fn test_move_word_right() {
        let buf = buf_with("hello world foo");
        let mut c = Cursor::new();

        c.move_word_right(&buf);
        assert_eq!(c.col, 6); // after "hello "

        c.move_word_right(&buf);
        assert_eq!(c.col, 12); // after "world "

        c.move_word_right(&buf);
        assert_eq!(c.col, 15); // end of "foo"
    }

    #[test]
    fn test_smart_home() {
        let buf = buf_with("    indented");
        let mut c = Cursor::new();
        c.col = 10; // middle of "indented"
        c.desired_col = 10;

        // col > first_non_ws(4) → go to first_non_ws
        c.move_home(&buf);
        assert_eq!(c.col, 4);

        // col == first_non_ws → go to 0
        c.move_home(&buf);
        assert_eq!(c.col, 0);

        // col == 0 → go to first_non_ws
        c.move_home(&buf);
        assert_eq!(c.col, 4);
    }

    #[test]
    fn test_move_end() {
        let buf = buf_with("hello\nworld");
        let mut c = Cursor::new();
        c.move_end(&buf);
        assert_eq!(c.col, 5);
    }

    #[test]
    fn test_page_up_down() {
        let buf = buf_with("a\nb\nc\nd\ne\nf\ng\nh\ni\nj");
        let mut c = Cursor::new();
        c.set_position(5, 0, &buf);

        c.move_page_up(&buf, 3);
        assert_eq!(c.line, 2);

        c.move_page_down(&buf, 3);
        assert_eq!(c.line, 5);

        // Page down past end
        c.move_page_down(&buf, 100);
        assert_eq!(c.line, 9);

        // Page up past start
        c.move_page_up(&buf, 100);
        assert_eq!(c.line, 0);
    }

    #[test]
    fn test_move_to_start_end() {
        let buf = buf_with("hello\nworld\nfoo");
        let mut c = Cursor::new();
        c.set_position(1, 3, &buf);

        c.move_to_end(&buf);
        assert_eq!(c.line, 2);
        assert_eq!(c.col, 3); // "foo" length

        c.move_to_start();
        assert_eq!(c.line, 0);
        assert_eq!(c.col, 0);
    }

    #[test]
    fn test_utf8_movement() {
        // "café" = c(1) a(1) f(1) é(2) = 5 bytes
        let buf = buf_with("café");
        let mut c = Cursor::new();

        c.move_right(&buf); // past 'c' → col 1
        assert_eq!(c.col, 1);
        c.move_right(&buf); // past 'a' → col 2
        assert_eq!(c.col, 2);
        c.move_right(&buf); // past 'f' → col 3
        assert_eq!(c.col, 3);
        c.move_right(&buf); // past 'é' (2 bytes) → col 5
        assert_eq!(c.col, 5);

        c.move_left(&buf); // back to start of 'é' → col 3
        assert_eq!(c.col, 3);
        c.move_left(&buf); // back to 'f' → col 2
        assert_eq!(c.col, 2);
    }

    #[test]
    fn test_byte_offset() {
        let buf = buf_with("ab\ncd\nef");
        let mut c = Cursor::new();

        // line 0, col 0 → offset 0
        assert_eq!(c.byte_offset(&buf), 0);

        c.set_position(1, 1, &buf);
        // line 1 starts at byte 3, col 1 → offset 4
        assert_eq!(c.byte_offset(&buf), 4);

        c.set_position(2, 2, &buf);
        // line 2 starts at byte 6, col 2 → offset 8
        assert_eq!(c.byte_offset(&buf), 8);
    }

    #[test]
    fn test_clamp_after_shrink() {
        let mut buf = Buffer::new();
        buf.insert(0, "hello\nworld\nfoo");
        let mut c = Cursor::new();
        c.line = 2;
        c.col = 3;
        c.desired_col = 3;

        // Delete "world\nfoo" leaving only "hello"
        buf.delete(5, 10);

        c.clamp(&buf);
        assert_eq!(c.line, 0);
        assert_eq!(c.col, 3); // 3 < 5 ("hello" length), so col stays at 3
    }

    #[test]
    fn test_set_position_clamps() {
        let buf = buf_with("short");
        let mut c = Cursor::new();
        c.set_position(100, 100, &buf);
        assert_eq!(c.line, 0);
        assert_eq!(c.col, 5);
    }

    #[test]
    fn test_move_word_left_wraps_line() {
        let buf = buf_with("hello\nworld");
        let mut c = Cursor::new();
        c.line = 1;
        c.col = 0;
        c.desired_col = 0;

        c.move_word_left(&buf);
        assert_eq!(c.line, 0);
        assert_eq!(c.col, 5);
    }

    #[test]
    fn test_move_word_right_wraps_line() {
        let buf = buf_with("hello\nworld");
        let mut c = Cursor::new();
        c.col = 5; // end of "hello"
        c.desired_col = 5;

        c.move_word_right(&buf);
        assert_eq!(c.line, 1);
        assert_eq!(c.col, 0);
    }
}
