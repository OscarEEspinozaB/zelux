mod terminal;

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

    let msg = format!(
        "Zelux — Terminal size: {}x{} | Color mode: {:?}\r\nPress any key to exit...",
        w, h, color_mode,
    );
    terminal::write_all(msg.as_bytes());
    terminal::flush();

    // Block until a keypress arrives
    loop {
        if term.read_byte().is_some() {
            break;
        }
    }

    // Terminal::drop restores everything
    drop(term);
}
