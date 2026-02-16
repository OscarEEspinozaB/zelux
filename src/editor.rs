use std::path::Path;

use crate::buffer::Buffer;
use crate::cursor::Cursor;
use crate::input::{self, Event, Key, KeyEvent, MouseButton};
use crate::render::{Color, Screen};
use crate::terminal::{self, ColorMode, Terminal};
use crate::undo::{CursorState, GroupContext, Operation, UndoStack};

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
    Find,
    Replace,
    ReplaceWith(String),
}

// ---------------------------------------------------------------------------
// Search state
// ---------------------------------------------------------------------------

struct SearchState {
    pattern: String,
    matches: Vec<(usize, usize)>, // (byte_start, byte_end)
    current: Option<usize>,       // index into matches
}

struct Prompt {
    label: String,
    input: String,
    cursor_pos: usize, // byte offset within input
    action: PromptAction,
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Selection {
    anchor: usize, // byte offset where selection started
    head: usize,   // byte offset at cursor end
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

    // Selection & clipboard
    selection: Option<Selection>,
    clipboard: String,

    // Active prompt (mini-prompt for Open, Save As, etc.)
    prompt: Option<Prompt>,

    // Undo/redo
    undo_stack: UndoStack,

    // Search
    search: Option<SearchState>,

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
            selection: None,
            clipboard: String::new(),
            prompt: None,
            undo_stack: UndoStack::new(),
            search: None,
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
            selection: None,
            clipboard: String::new(),
            prompt: None,
            undo_stack: UndoStack::new(),
            search: None,
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

                // Line content (with selection highlighting)
                let line_text = self.buffer.get_line(file_line).unwrap_or_default();
                let line_start_byte = self.buffer.line_start(file_line).unwrap_or(0);
                let sel_range = self.selection_range();
                let mut display_col: usize = 0;
                let mut byte_offset_in_line: usize = 0;
                for ch in line_text.chars() {
                    if display_col >= self.scroll_col {
                        let screen_col = display_col - self.scroll_col + self.gutter_width;
                        if screen_col >= screen_width {
                            break;
                        }
                        let char_byte = line_start_byte + byte_offset_in_line;
                        let is_selected =
                            sel_range.is_some_and(|(s, e)| char_byte >= s && char_byte < e);
                        let (fg, bg, bold) = if is_selected {
                            (Color::Ansi(0), Color::Ansi(7), true)
                        } else if let Some(is_current) = self.match_at_byte(char_byte) {
                            if is_current {
                                (Color::Ansi(0), Color::Ansi(6), true) // cyan bg
                            } else {
                                (Color::Ansi(0), Color::Ansi(3), false) // yellow bg
                            }
                        } else {
                            (Color::Default, Color::Default, false)
                        };
                        self.screen
                            .put_char(screen_row, screen_col, ch, fg, bg, bold);
                    }
                    byte_offset_in_line += ch.len_utf8();
                    display_col += 1;
                }
                // Fill remaining with spaces (selected if selection extends past EOL)
                let start_fill = display_col
                    .saturating_sub(self.scroll_col)
                    .saturating_add(self.gutter_width);
                let line_end_byte = line_start_byte + line_text.len();
                for col in start_fill..screen_width {
                    // Show selection highlight on trailing space if newline is selected
                    let is_trailing_selected = sel_range
                        .is_some_and(|(s, e)| line_end_byte >= s && line_end_byte < e)
                        && col == start_fill; // only first trailing cell
                    let (fg, bg, bold) = if is_trailing_selected {
                        (Color::Ansi(0), Color::Ansi(7), true)
                    } else {
                        (Color::Default, Color::Default, false)
                    };
                    self.screen.put_char(screen_row, col, ' ', fg, bg, bold);
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
                    self.delete_selection();
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

        let is_nav = matches!(
            &ke.key,
            Key::Up
                | Key::Down
                | Key::Left
                | Key::Right
                | Key::Home
                | Key::End
                | Key::PageUp
                | Key::PageDown
        );

        // Before navigation: start/continue selection if shift is held
        if is_nav && ke.shift {
            self.start_or_continue_selection();
        }

        match (&ke.key, ke.ctrl, ke.alt) {
            // -- Navigation (works with and without shift) --
            (Key::Up, false, _) => self.cursor.move_up(&self.buffer),
            (Key::Down, false, _) => self.cursor.move_down(&self.buffer),
            (Key::Left, false, _) => self.cursor.move_left(&self.buffer),
            (Key::Right, false, _) => self.cursor.move_right(&self.buffer),

            (Key::Left, true, _) => self.cursor.move_word_left(&self.buffer),
            (Key::Right, true, _) => self.cursor.move_word_right(&self.buffer),

            (Key::Home, false, _) => self.cursor.move_home(&self.buffer),
            (Key::End, false, _) => self.cursor.move_end(&self.buffer),

            (Key::Home, true, _) => self.cursor.move_to_start(),
            (Key::End, true, _) => self.cursor.move_to_end(&self.buffer),

            (Key::PageUp, false, _) => {
                let h = self.text_area_height();
                self.scroll_row = self.scroll_row.saturating_sub(h);
                self.cursor.move_page_up(&self.buffer, h);
            }
            (Key::PageDown, false, _) => {
                let h = self.text_area_height();
                let max_line = self.buffer.line_count().saturating_sub(1);
                self.scroll_row = (self.scroll_row + h).min(max_line);
                self.cursor.move_page_down(&self.buffer, h);
            }

            // -- Editing (delete selection first if active) --
            (Key::Char(ch), false, false) => {
                self.delete_selection();
                self.insert_char(*ch);
            }
            (Key::Enter, false, false) => {
                self.delete_selection();
                self.insert_newline();
            }
            (Key::Tab, false, false) => {
                self.delete_selection();
                self.insert_tab();
            }
            (Key::Backspace, false, false) => {
                if self.delete_selection().is_none() {
                    self.backspace();
                }
            }
            (Key::Delete, false, false) => {
                if self.delete_selection().is_none() {
                    self.delete_at_cursor();
                }
            }

            // -- Clipboard --
            (Key::Char('c'), true, false) => self.copy_selection(),
            (Key::Char('x'), true, false) => self.cut_selection(),
            (Key::Char('v'), true, false) => self.paste_clipboard(),
            (Key::Char('a'), true, false) => self.select_all(),

            // -- Commands --
            (Key::Char('s'), true, false) => self.save(),
            (Key::Char('q'), true, false) => self.quit(),

            // -- Undo/Redo --
            (Key::Char('z'), true, false) => {
                self.selection = None;
                let cs = self.cursor_state();
                if let Some(restored) = self.undo_stack.undo(&mut self.buffer, cs) {
                    self.restore_cursor(restored);
                    self.set_message("Undo", MessageType::Info);
                } else {
                    self.set_message("Nothing to undo", MessageType::Warning);
                }
            }
            (Key::Char('y'), true, false) => {
                self.selection = None;
                if let Some(restored) = self.undo_stack.redo(&mut self.buffer) {
                    self.restore_cursor(restored);
                    self.set_message("Redo", MessageType::Info);
                } else {
                    self.set_message("Nothing to redo", MessageType::Warning);
                }
            }

            // -- Search --
            (Key::Char('f'), true, false) => {
                self.open_find_prompt(PromptAction::Find);
            }
            (Key::Char('h'), true, false) => {
                self.open_find_prompt(PromptAction::Replace);
            }
            (Key::F(3), false, false) if !ke.shift => {
                self.search_next();
            }
            (Key::F(3), false, false) if ke.shift => {
                self.search_prev();
            }

            // -- File --
            (Key::Char('o'), true, false) => {
                self.start_prompt("Open: ", PromptAction::OpenFile);
            }

            _ => {}
        }

        // After navigation: extend or clear selection
        if is_nav {
            if ke.shift {
                self.extend_selection();
            } else {
                self.selection = None;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Selection helpers
    // -----------------------------------------------------------------------

    fn start_or_continue_selection(&mut self) {
        if self.selection.is_none() {
            let offset = self.cursor.byte_offset(&self.buffer);
            self.selection = Some(Selection {
                anchor: offset,
                head: offset,
            });
        }
    }

    fn extend_selection(&mut self) {
        if let Some(ref mut sel) = self.selection {
            sel.head = self.cursor.byte_offset(&self.buffer);
        }
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection.map(|sel| {
            let start = sel.anchor.min(sel.head);
            let end = sel.anchor.max(sel.head);
            (start, end)
        })
    }

    /// Delete the selected text, reposition cursor to selection start, clear selection.
    /// Returns the deleted text if there was a selection.
    fn delete_selection(&mut self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        if start == end {
            self.selection = None;
            return None;
        }
        let before = self.cursor_state();
        let deleted = self.buffer.slice(start, end);
        self.buffer.delete(start, end - start);
        self.undo_stack.record(
            Operation::Delete {
                pos: start,
                text: deleted.clone(),
            },
            before,
            GroupContext::Other,
        );
        // Reposition cursor to selection start
        let line = self.buffer.byte_to_line(start);
        let line_start = self.buffer.line_start(line).unwrap_or(0);
        let col = start - line_start;
        self.cursor.set_position(line, col, &self.buffer);
        self.selection = None;
        Some(deleted)
    }

    fn copy_selection(&mut self) {
        if let Some((start, end)) = self.selection_range() {
            if start == end {
                // No selection: copy current line
                self.copy_current_line();
                return;
            }
            let text = self.buffer.slice(start, end);
            let len = text.chars().count();
            self.clipboard = text.clone();
            terminal::set_clipboard_osc52(&text);
            self.set_message(&format!("Copied {} chars", len), MessageType::Info);
        } else {
            // No selection: copy current line
            self.copy_current_line();
        }
    }

    fn copy_current_line(&mut self) {
        let line_text = self.buffer.get_line(self.cursor.line).unwrap_or_default();
        let text = format!("{}\n", line_text);
        let len = line_text.chars().count();
        self.clipboard = text.clone();
        terminal::set_clipboard_osc52(&self.clipboard);
        self.set_message(&format!("Copied line ({} chars)", len), MessageType::Info);
    }

    fn cut_selection(&mut self) {
        if let Some((start, end)) = self.selection_range() {
            if start == end {
                self.cut_current_line();
                return;
            }
            let text = self.delete_selection().unwrap_or_default();
            let len = text.chars().count();
            self.clipboard = text.clone();
            terminal::set_clipboard_osc52(&text);
            self.set_message(&format!("Cut {} chars", len), MessageType::Info);
        } else {
            self.cut_current_line();
        }
    }

    fn cut_current_line(&mut self) {
        let before = self.cursor_state();
        let line = self.cursor.line;
        let line_start = self.buffer.line_start(line).unwrap_or(0);
        let line_end = self.buffer.line_end(line).unwrap_or(0);
        // Include the newline if not the last line
        let end = if line + 1 < self.buffer.line_count() {
            line_end + 1
        } else {
            line_end
        };
        let text = self.buffer.slice(line_start, end);
        let len = text.chars().count();
        self.buffer.delete(line_start, end - line_start);
        self.undo_stack.record(
            Operation::Delete {
                pos: line_start,
                text: text.clone(),
            },
            before,
            GroupContext::Cut,
        );
        self.cursor.clamp(&self.buffer);
        self.cursor.col = 0;
        self.cursor.desired_col = 0;
        self.clipboard = text.clone();
        terminal::set_clipboard_osc52(&text);
        self.set_message(&format!("Cut line ({} chars)", len), MessageType::Info);
    }

    fn paste_clipboard(&mut self) {
        if self.clipboard.is_empty() {
            self.set_message("Clipboard is empty", MessageType::Warning);
            return;
        }
        // Delete selection if active
        self.delete_selection();
        let text = self.clipboard.clone();
        self.handle_paste(&text);
    }

    fn select_all(&mut self) {
        let len = self.buffer.len();
        self.selection = Some(Selection {
            anchor: 0,
            head: len,
        });
        self.cursor.move_to_end(&self.buffer);
    }

    // -----------------------------------------------------------------------
    // Undo helpers
    // -----------------------------------------------------------------------

    fn cursor_state(&self) -> CursorState {
        CursorState {
            line: self.cursor.line,
            col: self.cursor.col,
            desired_col: self.cursor.desired_col,
        }
    }

    fn restore_cursor(&mut self, state: CursorState) {
        self.cursor.line = state.line;
        self.cursor.col = state.col;
        self.cursor.desired_col = state.desired_col;
        self.cursor.clamp(&self.buffer);
    }

    // -----------------------------------------------------------------------
    // Editing operations
    // -----------------------------------------------------------------------

    fn insert_char(&mut self, ch: char) {
        let before = self.cursor_state();
        let pos = self.cursor.byte_offset(&self.buffer);
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        self.buffer.insert(pos, s);
        self.undo_stack.record(
            Operation::Insert {
                pos,
                text: s.to_string(),
            },
            before,
            GroupContext::Typing,
        );
        self.cursor.move_right(&self.buffer);
    }

    fn insert_newline(&mut self) {
        let before = self.cursor_state();
        let pos = self.cursor.byte_offset(&self.buffer);
        self.buffer.insert(pos, "\n");
        self.undo_stack.record(
            Operation::Insert {
                pos,
                text: "\n".to_string(),
            },
            before,
            GroupContext::Other,
        );
        self.cursor.move_right(&self.buffer);
    }

    fn insert_tab(&mut self) {
        let before = self.cursor_state();
        let pos = self.cursor.byte_offset(&self.buffer);
        self.buffer.insert(pos, "    ");
        self.undo_stack.record(
            Operation::Insert {
                pos,
                text: "    ".to_string(),
            },
            before,
            GroupContext::Other,
        );
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
        let before = self.cursor_state();
        // Move cursor left first (handles UTF-8 boundaries)
        self.cursor.move_left(&self.buffer);
        let new_pos = self.cursor.byte_offset(&self.buffer);
        let delete_len = pos - new_pos;
        let deleted = self.buffer.slice(new_pos, pos);
        self.buffer.delete(new_pos, delete_len);
        self.undo_stack.record(
            Operation::Delete {
                pos: new_pos,
                text: deleted,
            },
            before,
            GroupContext::Deleting,
        );
    }

    fn delete_at_cursor(&mut self) {
        let pos = self.cursor.byte_offset(&self.buffer);
        if pos >= self.buffer.len() {
            return;
        }
        // Find the length of the character at cursor position
        if let Some(ch) = self.buffer.char_at(pos) {
            let before = self.cursor_state();
            let char_len = ch.len_utf8();
            let deleted = self.buffer.slice(pos, pos + char_len);
            self.buffer.delete(pos, char_len);
            self.undo_stack.record(
                Operation::Delete { pos, text: deleted },
                before,
                GroupContext::Deleting,
            );
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
                self.undo_stack.mark_saved(self.cursor_state());
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
        self.selection = None;

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
        let before = self.cursor_state();
        let pos = self.cursor.byte_offset(&self.buffer);
        self.buffer.insert(pos, text);
        self.undo_stack.record(
            Operation::Insert {
                pos,
                text: text.to_string(),
            },
            before,
            GroupContext::Paste,
        );
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
    // Search
    // -----------------------------------------------------------------------

    fn open_find_prompt(&mut self, action: PromptAction) {
        // Pre-fill with selection text (if short, single-line) or last search pattern
        let prefill = self.prefill_search_text();
        let label = match action {
            PromptAction::Replace | PromptAction::ReplaceWith(_) => "Find: ",
            _ => "Find: ",
        };
        self.prompt = Some(Prompt {
            label: label.to_string(),
            input: prefill.clone(),
            cursor_pos: prefill.len(),
            action,
        });
        self.message = None;
        // Trigger incremental search if prefill is non-empty
        if !prefill.is_empty() {
            self.update_search(&prefill);
        }
    }

    fn prefill_search_text(&self) -> String {
        // Use selection if it's short and single-line
        if let Some((start, end)) = self.selection_range()
            && start != end
        {
            let text = self.buffer.slice(start, end);
            if !text.contains('\n') && text.len() <= 100 {
                return text;
            }
        }
        // Fall back to last search pattern
        if let Some(ref search) = self.search {
            return search.pattern.clone();
        }
        String::new()
    }

    fn update_search(&mut self, pattern: &str) {
        if pattern.is_empty() {
            self.search = None;
            return;
        }
        let text = self.buffer.text();
        let matches = find_all_matches(&text, pattern);
        let cursor_byte = self.cursor.byte_offset(&self.buffer);

        // Find nearest match at or after cursor
        let current = if matches.is_empty() {
            None
        } else {
            let idx = matches
                .iter()
                .position(|(start, _)| *start >= cursor_byte)
                .unwrap_or(0);
            // Jump cursor to this match
            self.jump_to_byte(matches[idx].0);
            Some(idx)
        };

        self.search = Some(SearchState {
            pattern: pattern.to_string(),
            matches,
            current,
        });
    }

    fn search_next(&mut self) {
        let (total, next_idx, byte_pos) = {
            let search = match self.search {
                Some(ref s) if !s.matches.is_empty() => s,
                _ => {
                    self.set_message("No search pattern", MessageType::Warning);
                    return;
                }
            };
            let total = search.matches.len();
            let next = match search.current {
                Some(i) => (i + 1) % total,
                None => 0,
            };
            (total, next, search.matches[next].0)
        };
        self.jump_to_byte(byte_pos);
        self.search.as_mut().unwrap().current = Some(next_idx);
        self.set_message(
            &format!("Match {} of {}", next_idx + 1, total),
            MessageType::Info,
        );
    }

    fn search_prev(&mut self) {
        let (total, prev_idx, byte_pos) = {
            let search = match self.search {
                Some(ref s) if !s.matches.is_empty() => s,
                _ => {
                    self.set_message("No search pattern", MessageType::Warning);
                    return;
                }
            };
            let total = search.matches.len();
            let prev = match search.current {
                Some(i) => {
                    if i == 0 {
                        total - 1
                    } else {
                        i - 1
                    }
                }
                None => total - 1,
            };
            (total, prev, search.matches[prev].0)
        };
        self.jump_to_byte(byte_pos);
        self.search.as_mut().unwrap().current = Some(prev_idx);
        self.set_message(
            &format!("Match {} of {}", prev_idx + 1, total),
            MessageType::Info,
        );
    }

    fn jump_to_byte(&mut self, byte_pos: usize) {
        let line = self.buffer.byte_to_line(byte_pos);
        let line_start = self.buffer.line_start(line).unwrap_or(0);
        let col = byte_pos - line_start;
        self.cursor.set_position(line, col, &self.buffer);
    }

    fn execute_replace_all(&mut self, find_pattern: &str, replacement: &str) {
        let text = self.buffer.text();
        let matches = find_all_matches(&text, find_pattern);
        if matches.is_empty() {
            self.set_message("No matches to replace", MessageType::Warning);
            return;
        }
        let count = matches.len();

        // Replace in reverse order to preserve byte offsets
        for &(start, end) in matches.iter().rev() {
            let before = self.cursor_state();
            let deleted = self.buffer.slice(start, end);
            self.buffer.delete(start, end - start);
            self.undo_stack.record(
                Operation::Delete {
                    pos: start,
                    text: deleted,
                },
                before,
                GroupContext::Other,
            );
            let before2 = self.cursor_state();
            self.buffer.insert(start, replacement);
            self.undo_stack.record(
                Operation::Insert {
                    pos: start,
                    text: replacement.to_string(),
                },
                before2,
                GroupContext::Other,
            );
        }

        // Clear search state after replace
        self.search = None;
        self.cursor.clamp(&self.buffer);
        self.set_message(
            &format!("Replaced {} occurrences", count),
            MessageType::Info,
        );
    }

    /// Check if a byte position falls within any search match.
    /// Returns Some(is_current_match) if in a match, None otherwise.
    fn match_at_byte(&self, byte_pos: usize) -> Option<bool> {
        let search = self.search.as_ref()?;
        for (i, &(start, end)) in search.matches.iter().enumerate() {
            if byte_pos >= start && byte_pos < end {
                let is_current = search.current == Some(i);
                return Some(is_current);
            }
            if start > byte_pos {
                break; // matches are sorted, no need to continue
            }
        }
        None
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
        let mut input_changed = false;

        match (&ke.key, ke.ctrl, ke.alt) {
            (Key::Enter, false, false) => {
                // Take the prompt out to avoid borrow issues
                let prompt = self.prompt.take().unwrap();
                if prompt.input.is_empty() {
                    // Empty input — cancel
                    return;
                }
                self.execute_prompt(prompt);
                return;
            }
            (Key::Escape, _, _) => {
                // Keep search state so F3 still works
                self.prompt = None;
                return;
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
                        input_changed = true;
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
                        input_changed = true;
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
                    input_changed = true;
                }
            }
            _ => {}
        }

        // Incremental search: update matches when input changes in Find/Replace prompts
        if input_changed && let Some(ref prompt) = self.prompt {
            let is_search_prompt =
                matches!(prompt.action, PromptAction::Find | PromptAction::Replace);
            if is_search_prompt {
                let pattern = prompt.input.clone();
                self.update_search(&pattern);
            }
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
                        self.selection = None;
                        self.undo_stack.clear();
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
            PromptAction::Find => {
                // Finalize search, jump to current match
                self.update_search(&prompt.input.clone());
                if let Some(ref search) = self.search {
                    if search.matches.is_empty() {
                        self.set_message("No matches", MessageType::Warning);
                    } else {
                        let total = search.matches.len();
                        let current = search.current.map_or(0, |i| i + 1);
                        self.set_message(
                            &format!("Match {} of {}", current, total),
                            MessageType::Info,
                        );
                    }
                }
            }
            PromptAction::Replace => {
                // Save pattern, open "Replace with:" prompt
                let pattern = prompt.input;
                self.update_search(&pattern);
                if let Some(ref search) = self.search
                    && search.matches.is_empty()
                {
                    self.set_message("No matches", MessageType::Warning);
                    return;
                }
                self.start_prompt("Replace with: ", PromptAction::ReplaceWith(pattern));
            }
            PromptAction::ReplaceWith(ref find_pattern) => {
                let replacement = prompt.input;
                let find_pattern = find_pattern.clone();
                self.execute_replace_all(&find_pattern, &replacement);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Case-insensitive substring search. Returns non-overlapping byte ranges.
fn find_all_matches(text: &str, pattern: &str) -> Vec<(usize, usize)> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let text_lower = text.to_lowercase();
    let pattern_lower = pattern.to_lowercase();
    let pat_len = pattern_lower.len();
    let mut results = Vec::new();
    let mut start = 0;
    while start + pat_len <= text_lower.len() {
        if let Some(pos) = text_lower[start..].find(&pattern_lower) {
            let abs_pos = start + pos;
            results.push((abs_pos, abs_pos + pat_len));
            start = abs_pos + pat_len; // non-overlapping
        } else {
            break;
        }
    }
    results
}

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

    // -- Selection tests --

    #[test]
    fn test_selection_range_ordering() {
        // anchor < head
        let sel = Selection {
            anchor: 5,
            head: 10,
        };
        let (start, end) = {
            let s = sel.anchor.min(sel.head);
            let e = sel.anchor.max(sel.head);
            (s, e)
        };
        assert_eq!(start, 5);
        assert_eq!(end, 10);

        // anchor > head (backwards selection)
        let sel2 = Selection {
            anchor: 10,
            head: 5,
        };
        let (start2, end2) = {
            let s = sel2.anchor.min(sel2.head);
            let e = sel2.anchor.max(sel2.head);
            (s, e)
        };
        assert_eq!(start2, 5);
        assert_eq!(end2, 10);
    }

    #[test]
    fn test_delete_selection_repositions_cursor() {
        let mut buf = Buffer::new();
        buf.insert(0, "hello world");
        let mut cursor = Cursor::new();
        cursor.set_position(0, 5, &buf);

        // Simulate selection of " world" (bytes 5..11)
        let sel = Selection {
            anchor: 5,
            head: 11,
        };
        let (start, end) = (sel.anchor.min(sel.head), sel.anchor.max(sel.head));
        let deleted = buf.slice(start, end);
        buf.delete(start, end - start);
        let line = buf.byte_to_line(start);
        let line_start = buf.line_start(line).unwrap_or(0);
        let col = start - line_start;
        cursor.set_position(line, col, &buf);

        assert_eq!(deleted, " world");
        assert_eq!(buf.text(), "hello");
        assert_eq!(cursor.line, 0);
        assert_eq!(cursor.col, 5);
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

    // -- Search tests --

    #[test]
    fn test_find_all_matches_basic() {
        let matches = find_all_matches("hello hello", "hello");
        assert_eq!(matches, vec![(0, 5), (6, 11)]);
    }

    #[test]
    fn test_find_all_matches_case_insensitive() {
        let matches = find_all_matches("Hello HELLO", "hello");
        assert_eq!(matches, vec![(0, 5), (6, 11)]);
    }

    #[test]
    fn test_find_all_matches_empty_pattern() {
        let matches = find_all_matches("hello", "");
        assert!(matches.is_empty());
    }

    #[test]
    fn test_find_all_matches_no_overlap() {
        let matches = find_all_matches("aaa", "aa");
        assert_eq!(matches, vec![(0, 2)]);
    }

    #[test]
    fn test_find_all_matches_utf8() {
        let matches = find_all_matches("café café", "café");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0], (0, 5)); // "café" = 5 bytes
        assert_eq!(matches[1], (6, 11)); // after space
    }
}
