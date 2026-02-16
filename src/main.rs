mod buffer;
mod cursor;
mod input;
mod render;
mod terminal;

use input::{Event, Key, KeyEvent, read_event};
use terminal::{Terminal, detect_color_mode};

fn main() {
    let color_mode = detect_color_mode();

    let mut term = match Terminal::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let (w, h) = term.size();

    terminal::clear_screen();
    terminal::move_cursor(1, 1);

    let header = format!(
        "Zelux — {}x{} | {:?} | Press Ctrl+Q to exit\r\n\r\n",
        w, h, color_mode,
    );
    terminal::write_all(header.as_bytes());
    terminal::flush();

    loop {
        // Check for terminal resize
        if term.check_resize() {
            let (w, h) = term.size();
            let msg = format!("[Resize: {}x{}]\r\n", w, h);
            terminal::write_all(msg.as_bytes());
            terminal::flush();
        }

        let event = read_event(&term);

        match &event {
            Event::None => continue,

            Event::Key(KeyEvent {
                key: Key::Char('q'),
                ctrl: true,
                ..
            }) => break,

            Event::Key(ke) => {
                let msg = format!("Key: {:?}\r\n", ke);
                terminal::write_all(msg.as_bytes());
                terminal::flush();
            }

            Event::Mouse(me) => {
                let msg = format!("Mouse: {:?}\r\n", me);
                terminal::write_all(msg.as_bytes());
                terminal::flush();
            }

            Event::Paste(text) => {
                let preview = if text.len() > 60 {
                    format!("{}...", &text[..60])
                } else {
                    text.clone()
                };
                let msg = format!("Paste ({} bytes): {:?}\r\n", text.len(), preview);
                terminal::write_all(msg.as_bytes());
                terminal::flush();
            }

            Event::Resize => {
                let (w, h) = term.size();
                let msg = format!("[Resize event: {}x{}]\r\n", w, h);
                terminal::write_all(msg.as_bytes());
                terminal::flush();
            }
        }
    }

    drop(term);
}
