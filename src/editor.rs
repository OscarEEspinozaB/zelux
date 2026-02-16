use std::path::Path;

use crate::buffer::Buffer;
use crate::cursor::Cursor;
use crate::input::{self, Event, Key, KeyEvent, MouseButton};
use crate::render::{Color, Screen};
use crate::terminal::{self, ColorMode, Terminal};

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageType {
    Info,
    Error,
    Warning,
}

// ---------------------------------------------------------------------------
// Prompt (mini-prompt for commands like Open, Save As, Find, etc.)
// ---------------------------------------------------------------------------

enum PromptAction {
    OpenFile,
    // Future: SaveAs, Find, GoToLine
}

struct Prompt {
    label: String,
    input: String,
    cursor_pos: usize, // byte offset within input
    action: PromptAction,
}

// ---------------------------------------------------------------------------
// Editor
// ---------------------------------------------------------------------------

pub struct Editor {
    buffer: Buffer,
    cursor: Cursor,
    terminal: Terminal,
    screen: Screen,
    color_mode: ColorMode,

    // Viewport
    scroll_row: usize,
    scroll_col: usize,

    // UI layout
    gutter_width: usize,
    status_height: usize,

    // Transient message
    message: Option<String>,
    message_type: MessageType,

    // Quit state
    quit_confirm: bool,

    // Active prompt (mini-prompt for Open, Save As, etc.)
    prompt: Option<Prompt>,

    running: bool,
}

impl Editor {
    /// Create a new editor with an empty buffer.
    pub fn new() -> Result<Self, String> {
        let color_mode = terminal::detect_color_mode();
        let mut terminal = Terminal::new()?;
        let (w, h) = terminal.size();

        let buffer = Buffer::new();
        let gutter_width = compute_gutter_width(buffer.line_count());

        Ok(Editor {
            buffer,
            cursor: Cursor::new(),
            screen: Screen::new(w as usize, h as usize),
            terminal,
            color_mode,
            scroll_row: 0,
            scroll_col: 0,
            gutter_width,
            status_height: 2,
            message: None,
            message_type: MessageType::Info,
            quit_confirm: false,
            prompt: None,
            running: true,
        })
    }

    /// Create a new editor and load a file.
    pub fn open(path: &Path) -> Result<Self, String> {
        let color_mode = terminal::detect_color_mode();
        let mut terminal = Terminal::new()?;
        let (w, h) = terminal.size();

        let buffer = Buffer::from_file(path)?;
        let gutter_width = compute_gutter_width(buffer.line_count());

        Ok(Editor {
            buffer,
            cursor: Cursor::new(),
            screen: Screen::new(w as usize, h as usize),
            terminal,
            color_mode,
            scroll_row: 0,
            scroll_col: 0,
            gutter_width,
            status_height: 2,
            message: None,
            message_type: MessageType::Info,
            quit_confirm: false,
            prompt: None,
            running: true,
        })
    }

    /// Run the main editor loop.
    pub fn run(&mut self) -> Result<(), String> {
        while self.running {
            // 1. Check for resize
            if self.terminal.check_resize() {
                let (w, h) = self.terminal.size();
                self.screen.resize(w as usize, h as usize);
                self.adjust_viewport();
            }

            // 2. Render
            self.render();

            // 3. Read event (blocks until input or timeout)
            let event = input::read_event(&self.terminal);

            // 4. Handle event
            if event != Event::None {
                self.handle_event(event);
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Viewport
    // -----------------------------------------------------------------------

    fn text_area_height(&self) -> usize {
        self.screen.height().saturating_sub(self.status_height)
    }

    fn text_area_width(&self) -> usize {
        self.screen.width().saturating_sub(self.gutter_width)
    }

    fn adjust_viewport(&mut self) {
        let h = self.text_area_height();
        let w = self.text_area_width();

        // Vertical scrolling
        if h > 0 {
            if self.cursor.line < self.scroll_row {
                self.scroll_row = self.cursor.line;
            } else if self.cursor.line >= self.scroll_row + h {
                self.scroll_row = self.cursor.line - h + 1;
            }
        }

        // Horizontal scrolling
        let display_col = self.cursor_display_col();
        if w > 0 {
            if display_col < self.scroll_col {
                self.scroll_col = display_col;
            } else if display_col >= self.scroll_col + w {
                self.scroll_col = display_col - w + 1;
            }
        }
    }

    fn cursor_display_col(&self) -> usize {
        let line_text = self.buffer.get_line(self.cursor.line).unwrap_or_default();
        byte_col_to_display_col(&line_text, self.cursor.col)
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    fn render(&mut self) {
        self.gutter_width = compute_gutter_width(self.buffer.line_count());
        self.adjust_viewport();

        let h = self.text_area_height();
        let screen_width = self.screen.width();

        // -- Text area + gutter --
        for screen_row in 0..h {
            let file_line = self.scroll_row + screen_row;

            if file_line < self.buffer.line_count() {
                // Gutter: right-aligned line number
                let num_str = format!("{}", file_line + 1);
                let pad = self.gutter_width.saturating_sub(num_str.len() + 1);
                let gutter_fg = Color::Color256(240); // dim gray
                let gutter_bg = Color::Default;

                // Pad
                for col in 0..pad {
                    self.screen
                        .put_char(screen_row, col, ' ', gutter_fg, gutter_bg, false);
                }
                // Number
                self.screen
                    .put_str(screen_row, pad, &num_str, gutter_fg, gutter_bg, false);
                // Separator space
                let sep_col = pad + num_str.len();
                if sep_col < self.gutter_width {
                    self.screen
                        .put_char(screen_row, sep_col, ' ', gutter_fg, gutter_bg, false);
                }

                // Line content
                let line_text = self.buffer.get_line(file_line).unwrap_or_default();
                let mut display_col: usize = 0;
                for ch in line_text.chars() {
                    if display_col >= self.scroll_col {
                        let screen_col = display_col - self.scroll_col + self.gutter_width;
                        if screen_col >= screen_width {
                            break;
                        }
                        self.screen.put_char(
                            screen_row,
                            screen_col,
                            ch,
                            Color::Default,
                            Color::Default,
                            false,
                        );
                    }
                    display_col += 1;
                }
                // Fill remaining with spaces
                let start_fill = display_col
                    .saturating_sub(self.scroll_col)
                    .saturating_add(self.gutter_width);
                for col in start_fill..screen_width {
                    self.screen.put_char(
                        screen_row,
                        col,
                        ' ',
                        Color::Default,
                        Color::Default,
                        false,
                    );
                }
            } else {
                // Tilde line (past end of file)
                self.screen.put_char(
                    screen_row,
                    0,
                    '~',
                    Color::Color256(240),
                    Color::Default,
                    false,
                );
                for col in 1..screen_width {
                    self.screen.put_char(
                        screen_row,
                        col,
                        ' ',
                        Color::Default,
                        Color::Default,
                        false,
                    );
                }
            }
        }

        // -- Status bar (inverted colors) --
        let status_row = h;
        if status_row < self.screen.height() {
            let status_fg = Color::Ansi(0); // black
            let status_bg = Color::Ansi(7); // white

            // Build status text
            let filename = self
                .buffer
                .file_path()
                .map(shorten_path)
                .unwrap_or_else(|| "[No Name]".to_string());
            let modified_marker = if self.buffer.is_modified() {
                " [+]"
            } else {
                ""
            };
            let color_str = match self.color_mode {
                ColorMode::TrueColor => "TrueColor",
                ColorMode::Color256 => "256color",
                ColorMode::Color16 => "16color",
            };
            let position = format!(
                "Ln {}, Col {}",
                self.cursor.line + 1,
                self.cursor_display_col() + 1,
            );

            let left = format!(" {}{}", filename, modified_marker);
            let right = format!("{} | {} ", position, color_str);

            // Fill status bar
            for col in 0..screen_width {
                self.screen
                    .put_char(status_row, col, ' ', status_fg, status_bg, true);
            }
            // Left side
            self.screen
                .put_str(status_row, 0, &left, status_fg, status_bg, true);
            // Right side
            let right_start = screen_width.saturating_sub(right.len());
            self.screen
                .put_str(status_row, right_start, &right, status_fg, status_bg, true);
        }

        // -- Message line --
        let msg_row = h + 1;
        if msg_row < self.screen.height() {
            // Fill with spaces first
            for col in 0..screen_width {
                self.screen
                    .put_char(msg_row, col, ' ', Color::Default, Color::Default, false);
            }

            if let Some(ref prompt) = self.prompt {
                // Render prompt: label (yellow) + input (default)
                let label_fg = Color::Ansi(3); // yellow
                self.screen
                    .put_str(msg_row, 1, &prompt.label, label_fg, Color::Default, false);
                let input_start = 1 + prompt.label.chars().count();
                self.screen.put_str(
                    msg_row,
                    input_start,
                    &prompt.input,
                    Color::Default,
                    Color::Default,
                    false,
                );

                // Show error message after the input if present
                if let Some(ref msg) = self.message {
                    let msg_fg = match self.message_type {
                        MessageType::Error => Color::Ansi(1),
                        MessageType::Warning => Color::Ansi(3),
                        _ => Color::Ansi(2),
                    };
                    let err_start = input_start + prompt.input.chars().count() + 2;
                    if err_start < screen_width {
                        self.screen
                            .put_str(msg_row, err_start, msg, msg_fg, Color::Default, false);
                    }
                }
            } else if let Some(ref msg) = self.message {
                let msg_fg = match self.message_type {
                    MessageType::Info => Color::Ansi(2),    // green
                    MessageType::Error => Color::Ansi(1),   // red
                    MessageType::Warning => Color::Ansi(3), // yellow
                };
                self.screen
                    .put_str(msg_row, 1, msg, msg_fg, Color::Default, false);
            }
        }

        // Flush the screen
        self.screen.flush(&self.color_mode);

        // Position the hardware cursor
        if let Some(ref prompt) = self.prompt {
            // Cursor on message line within prompt input
            let prompt_cursor_col = 1
                + prompt.label.chars().count()
                + prompt.input[..prompt.cursor_pos].chars().count();
            let msg_row_1based = (h + 1 + 1) as u16; // h+1 is msg_row, +1 for 1-based
            terminal::move_cursor(msg_row_1based, (prompt_cursor_col + 1) as u16);
        } else {
            let cursor_screen_row = self.cursor.line.saturating_sub(self.scroll_row);
            let cursor_display = self.cursor_display_col();
            let cursor_screen_col = cursor_display
                .saturating_sub(self.scroll_col)
                .saturating_add(self.gutter_width);

            terminal::move_cursor(
                (cursor_screen_row + 1) as u16,
                (cursor_screen_col + 1) as u16,
            );
        }
        terminal::flush();
    }

    // -----------------------------------------------------------------------
    // Event handling
    // -----------------------------------------------------------------------

    fn handle_event(&mut self, event: Event) {
        // Clear message on any event (except resize), but only when no prompt is active
        if self.prompt.is_none() {
            match &event {
                Event::Resize => {}
                _ => {
                    self.message = None;
                }
            }
        }

        match event {
            Event::Key(ke) => {
                if self.prompt.is_some() {
                    self.handle_prompt_key(ke);
                } else {
                    self.handle_key(ke);
                }
            }
            Event::Mouse(me) => {
                if self.prompt.is_none() && me.button == MouseButton::Left && me.pressed {
                    self.handle_mouse_click(me.col, me.row);
                }
            }
            Event::Paste(text) => {
                if self.prompt.is_some() {
                    // Insert pasted text into prompt input
                    if let Some(ref mut prompt) = self.prompt {
                        prompt.input.insert_str(prompt.cursor_pos, &text);
                        prompt.cursor_pos += text.len();
                    }
                } else {
                    self.handle_paste(&text);
                }
            }
            Event::Resize => {
                let (w, h) = self.terminal.size();
                self.screen.resize(w as usize, h as usize);
                self.adjust_viewport();
            }
            Event::None => {}
        }
    }

    fn handle_key(&mut self, ke: KeyEvent) {
        // Reset quit confirmation on any key that isn't Ctrl+Q
        if !(ke.ctrl && ke.key == Key::Char('q')) {
            self.quit_confirm = false;
        }

        match (&ke.key, ke.ctrl, ke.alt) {
            // -- Navigation --
            (Key::Up, false, false) => self.cursor.move_up(&self.buffer),
            (Key::Down, false, false) => self.cursor.move_down(&self.buffer),
            (Key::Left, false, false) => self.cursor.move_left(&self.buffer),
            (Key::Right, false, false) => self.cursor.move_right(&self.buffer),

            (Key::Left, true, false) => self.cursor.move_word_left(&self.buffer),
            (Key::Right, true, false) => self.cursor.move_word_right(&self.buffer),

            (Key::Home, false, false) => self.cursor.move_home(&self.buffer),
            (Key::End, false, false) => self.cursor.move_end(&self.buffer),

            (Key::Home, true, false) => self.cursor.move_to_start(),
            (Key::End, true, false) => self.cursor.move_to_end(&self.buffer),

            (Key::PageUp, false, false) => {
                let h = self.text_area_height();
                self.scroll_row = self.scroll_row.saturating_sub(h);
                self.cursor.move_page_up(&self.buffer, h);
            }
            (Key::PageDown, false, false) => {
                let h = self.text_area_height();
                let max_line = self.buffer.line_count().saturating_sub(1);
                self.scroll_row = (self.scroll_row + h).min(max_line);
                self.cursor.move_page_down(&self.buffer, h);
            }

            // -- Editing --
            (Key::Char(ch), false, false) => {
                self.insert_char(*ch);
            }
            (Key::Enter, false, false) => {
                self.insert_newline();
            }
            (Key::Tab, false, false) => {
                self.insert_tab();
            }
            (Key::Backspace, false, false) => {
                self.backspace();
            }
            (Key::Delete, false, false) => {
                self.delete_at_cursor();
            }

            // -- Commands --
            (Key::Char('s'), true, false) => self.save(),
            (Key::Char('q'), true, false) => self.quit(),

            // -- Placeholders --
            (Key::Char('z'), true, false) => {
                self.set_message("Undo not implemented yet", MessageType::Warning);
            }
            (Key::Char('c'), true, false) => {
                self.set_message("Copy not implemented yet", MessageType::Warning);
            }
            (Key::Char('v'), true, false) => {
                self.set_message("Paste not implemented yet", MessageType::Warning);
            }
            (Key::Char('x'), true, false) => {
                self.set_message("Cut not implemented yet", MessageType::Warning);
            }
            (Key::Char('f'), true, false) => {
                self.set_message("Find not implemented yet", MessageType::Warning);
            }
            (Key::Char('o'), true, false) => {
                self.start_prompt("Open: ", PromptAction::OpenFile);
            }

            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Editing operations
    // -----------------------------------------------------------------------

    fn insert_char(&mut self, ch: char) {
        let pos = self.cursor.byte_offset(&self.buffer);
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        self.buffer.insert(pos, s);
        self.cursor.move_right(&self.buffer);
    }

    fn insert_newline(&mut self) {
        let pos = self.cursor.byte_offset(&self.buffer);
        self.buffer.insert(pos, "\n");
        self.cursor.move_right(&self.buffer);
    }

    fn insert_tab(&mut self) {
        let pos = self.cursor.byte_offset(&self.buffer);
        self.buffer.insert(pos, "    ");
        // Move right 4 times for 4 spaces
        for _ in 0..4 {
            self.cursor.move_right(&self.buffer);
        }
    }

    fn backspace(&mut self) {
        let pos = self.cursor.byte_offset(&self.buffer);
        if pos == 0 {
            return;
        }
        // Move cursor left first (handles UTF-8 boundaries)
        self.cursor.move_left(&self.buffer);
        let new_pos = self.cursor.byte_offset(&self.buffer);
        let delete_len = pos - new_pos;
        self.buffer.delete(new_pos, delete_len);
    }

    fn delete_at_cursor(&mut self) {
        let pos = self.cursor.byte_offset(&self.buffer);
        if pos >= self.buffer.len() {
            return;
        }
        // Find the length of the character at cursor position
        if let Some(ch) = self.buffer.char_at(pos) {
            let char_len = ch.len_utf8();
            self.buffer.delete(pos, char_len);
            self.cursor.clamp(&self.buffer);
        }
    }

    // -----------------------------------------------------------------------
    // Commands
    // -----------------------------------------------------------------------

    fn save(&mut self) {
        if self.buffer.file_path().is_none() {
            self.set_message(
                "No file name — use save_to (not yet implemented)",
                MessageType::Error,
            );
            return;
        }
        match self.buffer.save() {
            Ok(()) => {
                self.buffer.mark_saved();
                self.set_message("Saved!", MessageType::Info);
            }
            Err(e) => {
                self.set_message(&format!("Save failed: {}", e), MessageType::Error);
            }
        }
    }

    fn quit(&mut self) {
        if self.buffer.is_modified() && !self.quit_confirm {
            self.quit_confirm = true;
            self.set_message(
                "Unsaved changes! Press Ctrl+Q again to quit without saving.",
                MessageType::Warning,
            );
            return;
        }
        self.running = false;
    }

    // -----------------------------------------------------------------------
    // Mouse
    // -----------------------------------------------------------------------

    fn handle_mouse_click(&mut self, col: u16, row: u16) {
        let screen_row = row as usize;
        let screen_col = col as usize;

        let h = self.text_area_height();
        if screen_row >= h {
            return; // Click on status bar or message line
        }

        let file_line = self.scroll_row + screen_row;
        if file_line >= self.buffer.line_count() {
            return; // Click past end of file
        }

        // Convert screen column to byte column
        if screen_col < self.gutter_width {
            return; // Click on gutter
        }
        let display_col = screen_col - self.gutter_width + self.scroll_col;

        // Convert display column to byte column
        let line_text = self.buffer.get_line(file_line).unwrap_or_default();
        let byte_col = display_col_to_byte_col(&line_text, display_col);

        self.cursor.set_position(file_line, byte_col, &self.buffer);
    }

    // -----------------------------------------------------------------------
    // Paste
    // -----------------------------------------------------------------------

    fn handle_paste(&mut self, text: &str) {
        let pos = self.cursor.byte_offset(&self.buffer);
        self.buffer.insert(pos, text);
        // Advance cursor past inserted text
        for _ in text.chars() {
            self.cursor.move_right(&self.buffer);
        }
    }

    // -----------------------------------------------------------------------
    // Messages
    // -----------------------------------------------------------------------

    fn set_message(&mut self, msg: &str, msg_type: MessageType) {
        self.message = Some(msg.to_string());
        self.message_type = msg_type;
    }

    // -----------------------------------------------------------------------
    // Prompt
    // -----------------------------------------------------------------------

    fn start_prompt(&mut self, label: &str, action: PromptAction) {
        self.prompt = Some(Prompt {
            label: label.to_string(),
            input: String::new(),
            cursor_pos: 0,
            action,
        });
        self.message = None;
    }

    fn handle_prompt_key(&mut self, ke: KeyEvent) {
        match (&ke.key, ke.ctrl, ke.alt) {
            (Key::Enter, false, false) => {
                // Take the prompt out to avoid borrow issues
                let prompt = self.prompt.take().unwrap();
                if prompt.input.is_empty() {
                    // Empty input — cancel
                    return;
                }
                self.execute_prompt(prompt);
            }
            (Key::Escape, _, _) => {
                self.prompt = None;
            }
            (Key::Backspace, false, false) => {
                if let Some(ref mut prompt) = self.prompt
                    && prompt.cursor_pos > 0
                {
                    let before = &prompt.input[..prompt.cursor_pos];
                    if let Some(ch) = before.chars().next_back() {
                        let len = ch.len_utf8();
                        let new_pos = prompt.cursor_pos - len;
                        prompt.input.drain(new_pos..prompt.cursor_pos);
                        prompt.cursor_pos = new_pos;
                    }
                }
            }
            (Key::Delete, false, false) => {
                if let Some(ref mut prompt) = self.prompt
                    && prompt.cursor_pos < prompt.input.len()
                {
                    let after = &prompt.input[prompt.cursor_pos..];
                    if let Some(ch) = after.chars().next() {
                        let len = ch.len_utf8();
                        prompt
                            .input
                            .drain(prompt.cursor_pos..prompt.cursor_pos + len);
                    }
                }
            }
            (Key::Left, false, false) => {
                if let Some(ref mut prompt) = self.prompt
                    && prompt.cursor_pos > 0
                {
                    let before = &prompt.input[..prompt.cursor_pos];
                    if let Some(ch) = before.chars().next_back() {
                        prompt.cursor_pos -= ch.len_utf8();
                    }
                }
            }
            (Key::Right, false, false) => {
                if let Some(ref mut prompt) = self.prompt
                    && prompt.cursor_pos < prompt.input.len()
                {
                    let after = &prompt.input[prompt.cursor_pos..];
                    if let Some(ch) = after.chars().next() {
                        prompt.cursor_pos += ch.len_utf8();
                    }
                }
            }
            (Key::Home, false, false) => {
                if let Some(ref mut prompt) = self.prompt {
                    prompt.cursor_pos = 0;
                }
            }
            (Key::End, false, false) => {
                if let Some(ref mut prompt) = self.prompt {
                    prompt.cursor_pos = prompt.input.len();
                }
            }
            (Key::Char(ch), false, false) => {
                if let Some(ref mut prompt) = self.prompt {
                    let mut buf = [0u8; 4];
                    let s = ch.encode_utf8(&mut buf);
                    prompt.input.insert_str(prompt.cursor_pos, s);
                    prompt.cursor_pos += s.len();
                }
            }
            _ => {}
        }
    }

    fn execute_prompt(&mut self, prompt: Prompt) {
        match prompt.action {
            PromptAction::OpenFile => {
                let path = Path::new(&prompt.input);
                match Buffer::from_file(path) {
                    Ok(buf) => {
                        let display_name = shorten_path(path);
                        self.buffer = buf;
                        self.cursor = Cursor::new();
                        self.scroll_row = 0;
                        self.scroll_col = 0;
                        self.gutter_width = compute_gutter_width(self.buffer.line_count());
                        self.set_message(&format!("Opened: {}", display_name), MessageType::Info);
                    }
                    Err(e) => {
                        // Keep prompt open so user can fix the path
                        self.prompt = Some(prompt);
                        self.set_message(&format!("Error: {}", e), MessageType::Error);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn compute_gutter_width(line_count: usize) -> usize {
    let digits = if line_count == 0 {
        1
    } else {
        let mut n = line_count;
        let mut d = 0;
        while n > 0 {
            d += 1;
            n /= 10;
        }
        d
    };
    // digits + 2 (one space before, one after), minimum 4
    (digits + 2).max(4)
}

/// Shorten a file path for display: replace $HOME prefix with `~`.
fn shorten_path(path: &Path) -> String {
    let full = path.to_string_lossy();
    if let Some(home) = std::env::var_os("HOME") {
        let home_str = home.to_string_lossy();
        if let Some(rest) = full.strip_prefix(home_str.as_ref()) {
            if rest.is_empty() {
                return "~".to_string();
            }
            if rest.starts_with('/') {
                return format!("~{}", rest);
            }
        }
    }
    full.into_owned()
}

/// Convert a byte column offset into a display column (character count).
fn byte_col_to_display_col(line: &str, byte_col: usize) -> usize {
    let clamped = byte_col.min(line.len());
    line[..clamped].chars().count()
}

/// Convert a display column (character index) back to a byte offset.
fn display_col_to_byte_col(line: &str, display_col: usize) -> usize {
    let mut byte_offset = 0;
    for (i, ch) in line.chars().enumerate() {
        if i >= display_col {
            break;
        }
        byte_offset += ch.len_utf8();
    }
    byte_offset
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_gutter_width() {
        assert_eq!(compute_gutter_width(1), 4); // 1 digit + 2 = 3, min 4
        assert_eq!(compute_gutter_width(9), 4); // 1 digit + 2 = 3, min 4
        assert_eq!(compute_gutter_width(10), 4); // 2 digits + 2 = 4
        assert_eq!(compute_gutter_width(99), 4); // 2 digits + 2 = 4
        assert_eq!(compute_gutter_width(100), 5); // 3 digits + 2 = 5
        assert_eq!(compute_gutter_width(999), 5);
        assert_eq!(compute_gutter_width(1000), 6); // 4 digits + 2 = 6
    }

    #[test]
    fn test_shorten_path() {
        // Path outside home stays as-is
        assert_eq!(shorten_path(Path::new("/etc/config")), "/etc/config");

        // Home itself becomes ~
        if let Some(home) = std::env::var_os("HOME") {
            let home_str = home.to_string_lossy().to_string();
            assert_eq!(shorten_path(Path::new(&home_str)), "~");

            // Subpath under home gets ~ prefix
            let sub = format!("{}/projects/zelux", home_str);
            assert_eq!(shorten_path(Path::new(&sub)), "~/projects/zelux");
        }
    }

    #[test]
    fn test_byte_col_to_display_col() {
        assert_eq!(byte_col_to_display_col("hello", 0), 0);
        assert_eq!(byte_col_to_display_col("hello", 3), 3);
        assert_eq!(byte_col_to_display_col("hello", 5), 5);

        // "café" = c(1) a(1) f(1) é(2) = 5 bytes
        assert_eq!(byte_col_to_display_col("café", 0), 0);
        assert_eq!(byte_col_to_display_col("café", 3), 3); // before 'é'
        assert_eq!(byte_col_to_display_col("café", 5), 4); // after 'é'
    }

    #[test]
    fn test_display_col_to_byte_col() {
        assert_eq!(display_col_to_byte_col("hello", 0), 0);
        assert_eq!(display_col_to_byte_col("hello", 3), 3);
        assert_eq!(display_col_to_byte_col("hello", 5), 5);

        // "café" = c(1) a(1) f(1) é(2) = 5 bytes
        assert_eq!(display_col_to_byte_col("café", 3), 3); // before 'é'
        assert_eq!(display_col_to_byte_col("café", 4), 5); // after 'é'
    }

    // -- Prompt tests --

    #[test]
    fn test_prompt_insert_char() {
        let mut prompt = Prompt {
            label: "Open: ".to_string(),
            input: String::new(),
            cursor_pos: 0,
            action: PromptAction::OpenFile,
        };

        // Insert 'a'
        prompt.input.insert_str(prompt.cursor_pos, "a");
        prompt.cursor_pos += 1;
        assert_eq!(prompt.input, "a");
        assert_eq!(prompt.cursor_pos, 1);

        // Insert 'b'
        prompt.input.insert_str(prompt.cursor_pos, "b");
        prompt.cursor_pos += 1;
        assert_eq!(prompt.input, "ab");
        assert_eq!(prompt.cursor_pos, 2);

        // Move cursor left, insert 'x' in the middle
        let before = &prompt.input[..prompt.cursor_pos];
        if let Some(ch) = before.chars().next_back() {
            prompt.cursor_pos -= ch.len_utf8();
        }
        prompt.input.insert_str(prompt.cursor_pos, "x");
        prompt.cursor_pos += 1;
        assert_eq!(prompt.input, "axb");
        assert_eq!(prompt.cursor_pos, 2);
    }

    #[test]
    fn test_prompt_backspace() {
        let mut prompt = Prompt {
            label: "Open: ".to_string(),
            input: "hello".to_string(),
            cursor_pos: 5,
            action: PromptAction::OpenFile,
        };

        // Backspace at end
        let before = &prompt.input[..prompt.cursor_pos];
        if let Some(ch) = before.chars().next_back() {
            let len = ch.len_utf8();
            let new_pos = prompt.cursor_pos - len;
            prompt.input.drain(new_pos..prompt.cursor_pos);
            prompt.cursor_pos = new_pos;
        }
        assert_eq!(prompt.input, "hell");
        assert_eq!(prompt.cursor_pos, 4);
    }

    #[test]
    fn test_prompt_delete() {
        let mut prompt = Prompt {
            label: "Open: ".to_string(),
            input: "hello".to_string(),
            cursor_pos: 0,
            action: PromptAction::OpenFile,
        };

        // Delete at start
        let after = &prompt.input[prompt.cursor_pos..];
        if let Some(ch) = after.chars().next() {
            let len = ch.len_utf8();
            prompt
                .input
                .drain(prompt.cursor_pos..prompt.cursor_pos + len);
        }
        assert_eq!(prompt.input, "ello");
        assert_eq!(prompt.cursor_pos, 0);
    }

    #[test]
    fn test_prompt_cursor_movement() {
        let mut prompt = Prompt {
            label: "Open: ".to_string(),
            input: "abc".to_string(),
            cursor_pos: 0,
            action: PromptAction::OpenFile,
        };

        // Right
        let after = &prompt.input[prompt.cursor_pos..];
        if let Some(ch) = after.chars().next() {
            prompt.cursor_pos += ch.len_utf8();
        }
        assert_eq!(prompt.cursor_pos, 1);

        // End
        prompt.cursor_pos = prompt.input.len();
        assert_eq!(prompt.cursor_pos, 3);

        // Home
        prompt.cursor_pos = 0;
        assert_eq!(prompt.cursor_pos, 0);

        // Left at start — should stay at 0
        if prompt.cursor_pos > 0 {
            let before = &prompt.input[..prompt.cursor_pos];
            if let Some(ch) = before.chars().next_back() {
                prompt.cursor_pos -= ch.len_utf8();
            }
        }
        assert_eq!(prompt.cursor_pos, 0);
    }

    #[test]
    fn test_prompt_utf8_navigation() {
        let mut prompt = Prompt {
            label: "Open: ".to_string(),
            input: "café".to_string(), // c(1) a(1) f(1) é(2) = 5 bytes
            cursor_pos: 5,             // at end
            action: PromptAction::OpenFile,
        };

        // Left from end — should move back over 'é' (2 bytes)
        let before = &prompt.input[..prompt.cursor_pos];
        if let Some(ch) = before.chars().next_back() {
            prompt.cursor_pos -= ch.len_utf8();
        }
        assert_eq!(prompt.cursor_pos, 3);

        // Backspace 'é' — should remove 2 bytes
        // Move cursor back to end to test backspace over 'é'
        prompt.cursor_pos = 5;
        let before3 = &prompt.input[..prompt.cursor_pos];
        if let Some(ch) = before3.chars().next_back() {
            let len = ch.len_utf8();
            let new_pos = prompt.cursor_pos - len;
            prompt.input.drain(new_pos..prompt.cursor_pos);
            prompt.cursor_pos = new_pos;
        }
        assert_eq!(prompt.input, "caf");
        assert_eq!(prompt.cursor_pos, 3);
    }
}
