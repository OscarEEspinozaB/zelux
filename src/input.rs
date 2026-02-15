use crate::terminal::Terminal;

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl KeyEvent {
    fn plain(key: Key) -> Self {
        KeyEvent {
            key,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    fn ctrl(key: Key) -> Self {
        KeyEvent {
            key,
            ctrl: true,
            alt: false,
            shift: false,
        }
    }

    fn alt(key: Key) -> Self {
        KeyEvent {
            key,
            ctrl: false,
            alt: true,
            shift: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub button: MouseButton,
    pub col: u16,
    pub row: u16,
    pub pressed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize,
    None,
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Read and decode one input event from the terminal.
///
/// Returns `Event::None` when no data is available (timeout).
pub fn read_event(term: &Terminal) -> Event {
    let byte = match term.read_byte() {
        Some(b) => b,
        None => return Event::None,
    };

    match byte {
        // ESC — start of escape sequence or standalone Escape / Alt+key
        0x1b => parse_escape(term),

        // Control characters
        0x0d => Event::Key(KeyEvent::plain(Key::Enter)),
        0x09 => Event::Key(KeyEvent::plain(Key::Tab)),
        0x00 => Event::Key(KeyEvent::ctrl(Key::Char(' '))),

        // Ctrl+A .. Ctrl+Z (except 0x09=Tab, 0x0d=Enter)
        0x01..=0x1a => {
            let ch = (byte + b'a' - 1) as char;
            Event::Key(KeyEvent::ctrl(Key::Char(ch)))
        }

        // Other control chars we don't map
        0x1c..=0x1f => Event::None,

        // Backspace
        0x7f => Event::Key(KeyEvent::plain(Key::Backspace)),

        // Printable ASCII
        0x20..=0x7e => Event::Key(KeyEvent::plain(Key::Char(byte as char))),

        // UTF-8 multi-byte lead
        0xc0..=0xff => decode_utf8(byte, term),

        _ => Event::None,
    }
}

// ---------------------------------------------------------------------------
// ESC sequence handling
// ---------------------------------------------------------------------------

fn parse_escape(term: &Terminal) -> Event {
    // Try to read the next byte. If nothing comes, it's a lone Escape.
    let next = match term.read_byte() {
        Some(b) => b,
        None => return Event::Key(KeyEvent::plain(Key::Escape)),
    };

    match next {
        b'[' => parse_csi(term),
        b'O' => parse_ss3(term),
        // Alt + printable character
        0x20..=0x7e => Event::Key(KeyEvent::alt(Key::Char(next as char))),
        _ => Event::Key(KeyEvent::plain(Key::Escape)),
    }
}

// ---------------------------------------------------------------------------
// CSI sequence parser (\x1b[ ...)
// ---------------------------------------------------------------------------

fn parse_csi(term: &Terminal) -> Event {
    let mut params = [0u16; 8];
    let mut param_count: usize = 0;
    let mut current: u16 = 0;
    let mut has_digit = false;
    let mut sgr_prefix = false;

    loop {
        let b = match term.read_byte() {
            Some(b) => b,
            None => return Event::None,
        };

        match b {
            // SGR mouse prefix
            b'<' => {
                sgr_prefix = true;
            }

            // Parameter digit
            b'0'..=b'9' => {
                current = current.saturating_mul(10).saturating_add((b - b'0') as u16);
                has_digit = true;
            }

            // Parameter separator
            b';' => {
                if param_count < params.len() {
                    params[param_count] = current;
                    param_count += 1;
                }
                current = 0;
                has_digit = false;
            }

            // Final byte — terminates the sequence
            0x40..=0x7e => {
                // Push last parameter
                if (has_digit || param_count > 0) && param_count < params.len() {
                    params[param_count] = current;
                    param_count += 1;
                }

                // SGR mouse: \x1b[<btn;col;rowM  or  \x1b[<btn;col;rowm
                if sgr_prefix && (b == b'M' || b == b'm') && param_count >= 3 {
                    return parse_sgr_mouse(params[0], params[1], params[2], b == b'M');
                }

                // Bracketed paste: \x1b[200~
                if b == b'~' && param_count == 1 && params[0] == 200 {
                    return read_bracketed_paste(term);
                }

                return decode_csi_final(b, &params[..param_count]);
            }

            _ => return Event::None,
        }
    }
}

/// Decode the final byte of a CSI sequence into an Event.
fn decode_csi_final(final_byte: u8, params: &[u16]) -> Event {
    // Extract modifier if present (param index 1 for letter-finals, last param for ~)
    let modifier = |idx: usize| -> (bool, bool, bool) {
        if idx < params.len() && params[idx] > 1 {
            decode_modifier(params[idx])
        } else {
            (false, false, false)
        }
    };

    match final_byte {
        // Arrow keys: \x1b[A .. \x1b[D  with optional modifier in params[1]
        b'A' => key_with_mod(Key::Up, modifier(1)),
        b'B' => key_with_mod(Key::Down, modifier(1)),
        b'C' => key_with_mod(Key::Right, modifier(1)),
        b'D' => key_with_mod(Key::Left, modifier(1)),

        // Home / End
        b'H' => key_with_mod(Key::Home, modifier(1)),
        b'F' => key_with_mod(Key::End, modifier(1)),

        // Tilde sequences: \x1b[N~ or \x1b[N;mod~
        b'~' if !params.is_empty() => {
            let mod_idx = if params.len() >= 2 { 1 } else { 99 };
            let mods = modifier(mod_idx);
            match params[0] {
                1 | 7 => key_with_mod(Key::Home, mods),
                3 => key_with_mod(Key::Delete, mods),
                4 | 8 => key_with_mod(Key::End, mods),
                5 => key_with_mod(Key::PageUp, mods),
                6 => key_with_mod(Key::PageDown, mods),
                11 => key_with_mod(Key::F(1), mods),
                12 => key_with_mod(Key::F(2), mods),
                13 => key_with_mod(Key::F(3), mods),
                14 => key_with_mod(Key::F(4), mods),
                15 => key_with_mod(Key::F(5), mods),
                17 => key_with_mod(Key::F(6), mods),
                18 => key_with_mod(Key::F(7), mods),
                19 => key_with_mod(Key::F(8), mods),
                20 => key_with_mod(Key::F(9), mods),
                21 => key_with_mod(Key::F(10), mods),
                23 => key_with_mod(Key::F(11), mods),
                24 => key_with_mod(Key::F(12), mods),
                _ => Event::None,
            }
        }

        _ => Event::None,
    }
}

/// Decode xterm modifier encoding: value = 1 + (shift?1:0) + (alt?2:0) + (ctrl?4:0)
fn decode_modifier(value: u16) -> (bool, bool, bool) {
    let v = value.saturating_sub(1) as u8;
    let shift = v & 1 != 0;
    let alt = v & 2 != 0;
    let ctrl = v & 4 != 0;
    (ctrl, alt, shift)
}

fn key_with_mod(key: Key, (ctrl, alt, shift): (bool, bool, bool)) -> Event {
    Event::Key(KeyEvent {
        key,
        ctrl,
        alt,
        shift,
    })
}

// ---------------------------------------------------------------------------
// SGR mouse: \x1b[<btn;col;rowM/m
// ---------------------------------------------------------------------------

fn parse_sgr_mouse(btn_bits: u16, col: u16, row: u16, pressed: bool) -> Event {
    let button = match btn_bits & 0x43 {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        64 => MouseButton::ScrollUp,
        65 => MouseButton::ScrollDown,
        _ => return Event::None,
    };

    Event::Mouse(MouseEvent {
        button,
        col: col.saturating_sub(1), // 1-based to 0-based
        row: row.saturating_sub(1),
        pressed,
    })
}

// ---------------------------------------------------------------------------
// Bracketed paste: read until \x1b[201~
// ---------------------------------------------------------------------------

fn read_bracketed_paste(term: &Terminal) -> Event {
    let mut buf = Vec::with_capacity(256);

    // We need to detect the ending sequence \x1b[201~
    // Use a simple state machine.
    while let Some(b) = term.read_byte() {
        buf.push(b);

        // Check for \x1b[201~ at the end of buffer
        if buf.len() >= 6 && buf[buf.len() - 6..] == *b"\x1b[201~" {
            buf.truncate(buf.len() - 6);
            break;
        }

        // Safety limit: 1MB paste
        if buf.len() > 1_048_576 {
            break;
        }
    }

    let text = String::from_utf8_lossy(&buf).into_owned();
    Event::Paste(text)
}

// ---------------------------------------------------------------------------
// SS3 sequences: \x1bO ...
// ---------------------------------------------------------------------------

fn parse_ss3(term: &Terminal) -> Event {
    let b = match term.read_byte() {
        Some(b) => b,
        None => return Event::None,
    };

    let key = match b {
        b'P' => Key::F(1),
        b'Q' => Key::F(2),
        b'R' => Key::F(3),
        b'S' => Key::F(4),
        b'H' => Key::Home,
        b'F' => Key::End,
        _ => return Event::None,
    };

    Event::Key(KeyEvent::plain(key))
}

// ---------------------------------------------------------------------------
// UTF-8 decoder
// ---------------------------------------------------------------------------

fn decode_utf8(lead: u8, term: &Terminal) -> Event {
    let (expected, mut codepoint) = if lead & 0xE0 == 0xC0 {
        (1, (lead & 0x1F) as u32)
    } else if lead & 0xF0 == 0xE0 {
        (2, (lead & 0x0F) as u32)
    } else if lead & 0xF8 == 0xF0 {
        (3, (lead & 0x07) as u32)
    } else {
        return Event::None; // invalid lead byte
    };

    for _ in 0..expected {
        match term.read_byte() {
            Some(b) if b & 0xC0 == 0x80 => {
                codepoint = (codepoint << 6) | (b & 0x3F) as u32;
            }
            _ => return Event::None,
        }
    }

    match char::from_u32(codepoint) {
        Some(ch) => Event::Key(KeyEvent::plain(Key::Char(ch))),
        None => Event::None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_modifier() {
        // 1 = no modifiers
        assert_eq!(decode_modifier(1), (false, false, false));
        // 2 = Shift
        assert_eq!(decode_modifier(2), (false, false, true));
        // 3 = Alt
        assert_eq!(decode_modifier(3), (false, true, false));
        // 5 = Ctrl
        assert_eq!(decode_modifier(5), (true, false, false));
        // 6 = Ctrl+Shift
        assert_eq!(decode_modifier(6), (true, false, true));
        // 8 = Ctrl+Alt+Shift
        assert_eq!(decode_modifier(8), (true, true, true));
    }

    #[test]
    fn test_key_event_constructors() {
        let plain = KeyEvent::plain(Key::Enter);
        assert!(!plain.ctrl && !plain.alt && !plain.shift);

        let ctrl = KeyEvent::ctrl(Key::Char('c'));
        assert!(ctrl.ctrl && !ctrl.alt && !ctrl.shift);

        let alt = KeyEvent::alt(Key::Char('x'));
        assert!(!alt.ctrl && alt.alt && !alt.shift);
    }

    #[test]
    fn test_decode_csi_arrows() {
        // \x1b[A = Up
        assert_eq!(
            decode_csi_final(b'A', &[]),
            Event::Key(KeyEvent::plain(Key::Up))
        );
        // \x1b[1;5C = Ctrl+Right
        assert_eq!(
            decode_csi_final(b'C', &[1, 5]),
            Event::Key(KeyEvent {
                key: Key::Right,
                ctrl: true,
                alt: false,
                shift: false,
            })
        );
    }

    #[test]
    fn test_decode_csi_tilde() {
        // \x1b[3~ = Delete
        assert_eq!(
            decode_csi_final(b'~', &[3]),
            Event::Key(KeyEvent::plain(Key::Delete))
        );
        // \x1b[5;2~ = Shift+PageUp
        assert_eq!(
            decode_csi_final(b'~', &[5, 2]),
            Event::Key(KeyEvent {
                key: Key::PageUp,
                ctrl: false,
                alt: false,
                shift: true,
            })
        );
        // \x1b[15~ = F5
        assert_eq!(
            decode_csi_final(b'~', &[15]),
            Event::Key(KeyEvent::plain(Key::F(5)))
        );
    }

    #[test]
    fn test_sgr_mouse() {
        assert_eq!(
            parse_sgr_mouse(0, 10, 5, true),
            Event::Mouse(MouseEvent {
                button: MouseButton::Left,
                col: 9,
                row: 4,
                pressed: true,
            })
        );
        assert_eq!(
            parse_sgr_mouse(65, 1, 1, true),
            Event::Mouse(MouseEvent {
                button: MouseButton::ScrollDown,
                col: 0,
                row: 0,
                pressed: true,
            })
        );
    }
}
