use std::fs;
use std::path::{Path, PathBuf};

const INITIAL_GAP: usize = 1024;

pub struct Buffer {
    data: Vec<u8>,
    gap_start: usize,
    gap_end: usize,
    lines: Vec<usize>,
    modified: bool,
    file_path: Option<PathBuf>,
}

impl Buffer {
    pub fn new() -> Buffer {
        let data = vec![0u8; INITIAL_GAP];
        Buffer {
            data,
            gap_start: 0,
            gap_end: INITIAL_GAP,
            lines: vec![0],
            modified: false,
            file_path: None,
        }
    }

    pub fn from_file(path: &Path) -> Result<Buffer, String> {
        let content = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
        let content_len = content.len();
        let gap_size = INITIAL_GAP.max(content_len / 4);
        let mut data = Vec::with_capacity(content_len + gap_size);
        data.extend_from_slice(&content);
        data.resize(content_len + gap_size, 0);

        let mut buf = Buffer {
            data,
            gap_start: content_len,
            gap_end: content_len + gap_size,
            lines: Vec::new(),
            modified: false,
            file_path: Some(path.to_path_buf()),
        };
        buf.rebuild_lines();
        Ok(buf)
    }

    pub fn save(&self) -> Result<(), String> {
        let path = self
            .file_path
            .as_ref()
            .ok_or_else(|| "No file path set".to_string())?;
        fs::write(path, self.text_bytes()).map_err(|e| format!("Failed to write file: {}", e))
    }

    pub fn save_to(&mut self, path: &Path) -> Result<(), String> {
        fs::write(path, self.text_bytes()).map_err(|e| format!("Failed to write file: {}", e))?;
        self.file_path = Some(path.to_path_buf());
        self.modified = false;
        Ok(())
    }

    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    pub fn is_modified(&self) -> bool {
        self.modified
    }

    pub fn mark_saved(&mut self) {
        self.modified = false;
    }

    // --- Text access ---

    pub fn len(&self) -> usize {
        self.data.len() - self.gap_len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get_line(&self, line: usize) -> Option<String> {
        if line >= self.lines.len() {
            return None;
        }
        let start = self.lines[line];
        let end = if line + 1 < self.lines.len() {
            // Line ends just before the '\n' that starts the next line
            self.lines[line + 1] - 1
        } else {
            self.len()
        };
        let mut result = Vec::with_capacity(end - start);
        for i in start..end {
            if let Some(b) = self.byte_at(i) {
                result.push(b);
            }
        }
        Some(String::from_utf8_lossy(&result).into_owned())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.text_bytes()).into_owned()
    }

    pub fn char_at(&self, byte_pos: usize) -> Option<char> {
        if byte_pos >= self.len() {
            return None;
        }
        let first = self.byte_at(byte_pos)?;
        let char_len = utf8_char_len(first);
        if char_len == 1 {
            return Some(first as char);
        }
        let mut bytes = [first, 0, 0, 0];
        for i in 1..char_len {
            bytes[i] = self.byte_at(byte_pos + i)?; // indexing both bytes[] and byte_pos offset
        }
        std::str::from_utf8(&bytes[..char_len])
            .ok()
            .and_then(|s| s.chars().next())
    }

    // --- Editing ---

    pub fn insert(&mut self, pos: usize, text: &str) {
        let pos = pos.min(self.len());
        let bytes = text.as_bytes();
        self.ensure_gap(bytes.len());
        self.move_gap(pos);
        self.data[self.gap_start..self.gap_start + bytes.len()].copy_from_slice(bytes);
        self.gap_start += bytes.len();
        self.modified = true;
        self.rebuild_lines();
    }

    pub fn delete(&mut self, pos: usize, len: usize) -> String {
        if len == 0 || pos >= self.len() {
            return String::new();
        }
        let len = len.min(self.len() - pos);
        // Collect the bytes being deleted
        let mut deleted = Vec::with_capacity(len);
        for i in pos..pos + len {
            if let Some(b) = self.byte_at(i) {
                deleted.push(b);
            }
        }
        self.move_gap(pos);
        self.gap_end += len;
        self.modified = true;
        self.rebuild_lines();
        String::from_utf8_lossy(&deleted).into_owned()
    }

    // --- Line info ---

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.lines.get(line).copied()
    }

    pub fn line_end(&self, line: usize) -> Option<usize> {
        if line >= self.lines.len() {
            return None;
        }
        if line + 1 < self.lines.len() {
            Some(self.lines[line + 1] - 1)
        } else {
            Some(self.len())
        }
    }

    pub fn byte_to_line(&self, byte_pos: usize) -> usize {
        // Binary search: find the last line whose start <= byte_pos
        match self.lines.binary_search(&byte_pos) {
            Ok(i) => i,
            Err(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
        }
    }

    // --- Internal ---

    fn gap_len(&self) -> usize {
        self.gap_end - self.gap_start
    }

    fn logical_to_physical(&self, pos: usize) -> usize {
        if pos < self.gap_start {
            pos
        } else {
            pos + self.gap_len()
        }
    }

    fn byte_at(&self, pos: usize) -> Option<u8> {
        if pos >= self.len() {
            return None;
        }
        Some(self.data[self.logical_to_physical(pos)])
    }

    fn move_gap(&mut self, pos: usize) {
        let pos = pos.min(self.len());
        if pos == self.gap_start {
            return;
        }
        if pos < self.gap_start {
            let count = self.gap_start - pos;
            let src = pos;
            let dst = self.gap_end - count;
            self.data.copy_within(src..src + count, dst);
            self.gap_start = pos;
            self.gap_end = dst;
        } else {
            // pos > gap_start
            let count = pos - self.gap_start;
            let src = self.gap_end;
            let dst = self.gap_start;
            self.data.copy_within(src..src + count, dst);
            self.gap_start = pos;
            self.gap_end = src + count;
        }
    }

    fn ensure_gap(&mut self, needed: usize) {
        if self.gap_len() >= needed {
            return;
        }
        let grow = INITIAL_GAP.max(self.len() / 4).max(needed);
        let old_len = self.data.len();
        let after_gap = old_len - self.gap_end;
        self.data.resize(old_len + grow, 0);
        // Move post-gap data to end
        if after_gap > 0 {
            let new_after_start = self.data.len() - after_gap;
            self.data
                .copy_within(self.gap_end..self.gap_end + after_gap, new_after_start);
            self.gap_end = new_after_start;
        } else {
            self.gap_end = self.data.len();
        }
    }

    fn rebuild_lines(&mut self) {
        self.lines.clear();
        self.lines.push(0);
        let total = self.len();
        for i in 0..total {
            if self.byte_at(i) == Some(b'\n') {
                self.lines.push(i + 1);
            }
        }
    }

    fn text_bytes(&self) -> Vec<u8> {
        let total = self.len();
        let mut result = Vec::with_capacity(total);
        result.extend_from_slice(&self.data[..self.gap_start]);
        result.extend_from_slice(&self.data[self.gap_end..]);
        result
    }
}

fn utf8_char_len(first_byte: u8) -> usize {
    if first_byte & 0x80 == 0 {
        1
    } else if first_byte & 0xE0 == 0xC0 {
        2
    } else if first_byte & 0xF0 == 0xE0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_empty_buffer() {
        let buf = Buffer::new();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert_eq!(buf.line_count(), 1);
        assert!(!buf.is_modified());
        assert_eq!(buf.text(), "");
        assert_eq!(buf.get_line(0), Some(String::new()));
        assert_eq!(buf.get_line(1), None);
    }

    #[test]
    fn test_insert_single_char() {
        let mut buf = Buffer::new();
        buf.insert(0, "a");
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.text(), "a");
        assert!(buf.is_modified());
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn test_insert_multiline() {
        let mut buf = Buffer::new();
        buf.insert(0, "hello\nworld\n");
        assert_eq!(buf.line_count(), 3);
        assert_eq!(buf.get_line(0), Some("hello".into()));
        assert_eq!(buf.get_line(1), Some("world".into()));
        assert_eq!(buf.get_line(2), Some(String::new()));
        assert_eq!(buf.text(), "hello\nworld\n");
    }

    #[test]
    fn test_insert_utf8() {
        let mut buf = Buffer::new();
        buf.insert(0, "café ñ 日本語");
        assert_eq!(buf.text(), "café ñ 日本語");
        assert_eq!(buf.char_at(0), Some('c'));
        assert_eq!(buf.char_at(3), Some('é'));
        // 'é' is 2 bytes, so 'café' = 5 bytes, then space at 5
        assert_eq!(buf.char_at(5), Some(' '));
    }

    #[test]
    fn test_delete_range() {
        let mut buf = Buffer::new();
        buf.insert(0, "hello world");
        let deleted = buf.delete(5, 6);
        assert_eq!(deleted, " world");
        assert_eq!(buf.text(), "hello");
    }

    #[test]
    fn test_delete_empty() {
        let mut buf = Buffer::new();
        buf.insert(0, "abc");
        let deleted = buf.delete(3, 5);
        assert_eq!(deleted, "");
        assert_eq!(buf.text(), "abc");
    }

    #[test]
    fn test_gap_movement() {
        let mut buf = Buffer::new();
        buf.insert(0, "abcdef");
        // Insert in middle forces gap move
        buf.insert(3, "XY");
        assert_eq!(buf.text(), "abcXYdef");
        // Insert at beginning
        buf.insert(0, "Z");
        assert_eq!(buf.text(), "ZabcXYdef");
        // Insert at end
        buf.insert(buf.len(), "!");
        assert_eq!(buf.text(), "ZabcXYdef!");
    }

    #[test]
    fn test_get_line() {
        let mut buf = Buffer::new();
        buf.insert(0, "first\nsecond\nthird");
        assert_eq!(buf.get_line(0), Some("first".into()));
        assert_eq!(buf.get_line(1), Some("second".into()));
        assert_eq!(buf.get_line(2), Some("third".into()));
        assert_eq!(buf.get_line(3), None);
    }

    #[test]
    fn test_byte_to_line() {
        let mut buf = Buffer::new();
        buf.insert(0, "ab\ncd\nef");
        // "ab\ncd\nef" -> lines at 0, 3, 6
        assert_eq!(buf.byte_to_line(0), 0); // 'a'
        assert_eq!(buf.byte_to_line(1), 0); // 'b'
        assert_eq!(buf.byte_to_line(2), 0); // '\n'
        assert_eq!(buf.byte_to_line(3), 1); // 'c'
        assert_eq!(buf.byte_to_line(5), 1); // '\n'
        assert_eq!(buf.byte_to_line(6), 2); // 'e'
    }

    #[test]
    fn test_line_start_end() {
        let mut buf = Buffer::new();
        buf.insert(0, "ab\ncd\nef");
        assert_eq!(buf.line_start(0), Some(0));
        assert_eq!(buf.line_end(0), Some(2)); // before '\n'
        assert_eq!(buf.line_start(1), Some(3));
        assert_eq!(buf.line_end(1), Some(5));
        assert_eq!(buf.line_start(2), Some(6));
        assert_eq!(buf.line_end(2), Some(8)); // end of buffer
    }

    #[test]
    fn test_file_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join("zelux_test_buffer.txt");
        let content = "Hello\nWorld\nTest\n";

        // Write test file
        {
            let mut f = fs::File::create(&path).unwrap();
            f.write_all(content.as_bytes()).unwrap();
        }

        let buf = Buffer::from_file(&path).unwrap();
        assert_eq!(buf.text(), content);
        assert_eq!(buf.line_count(), 4);
        assert!(!buf.is_modified());
        assert_eq!(buf.file_path(), Some(path.as_path()));

        // Save to different file
        let path2 = dir.join("zelux_test_buffer2.txt");
        let mut buf = buf;
        buf.save_to(&path2).unwrap();
        let buf2 = Buffer::from_file(&path2).unwrap();
        assert_eq!(buf2.text(), content);

        // Cleanup
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&path2);
    }

    #[test]
    fn test_modified_flag() {
        let mut buf = Buffer::new();
        assert!(!buf.is_modified());
        buf.insert(0, "x");
        assert!(buf.is_modified());
        buf.mark_saved();
        assert!(!buf.is_modified());
        buf.delete(0, 1);
        assert!(buf.is_modified());
    }

    #[test]
    fn test_large_insert() {
        let mut buf = Buffer::new();
        let large = "x".repeat(10240);
        buf.insert(0, &large);
        assert_eq!(buf.len(), 10240);
        assert_eq!(buf.text(), large);
        // Insert more at middle
        buf.insert(5000, "MIDDLE");
        assert_eq!(buf.len(), 10246);
        assert_eq!(buf.char_at(5000), Some('M'));
    }

    #[test]
    fn test_sequential_inserts() {
        let mut buf = Buffer::new();
        for c in "hello".chars() {
            let pos = buf.len();
            buf.insert(pos, &c.to_string());
        }
        assert_eq!(buf.text(), "hello");
    }
}
