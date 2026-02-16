mod buffer;
mod cursor;
mod editor;
mod input;
mod render;
mod terminal;
mod undo;

use std::env;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut editor = if args.len() > 1 {
        editor::Editor::open(Path::new(&args[1]))
    } else {
        editor::Editor::new()
    }
    .unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    });

    if let Err(e) = editor.run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
