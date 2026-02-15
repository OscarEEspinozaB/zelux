use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------------------
// libc FFI — zero external dependencies
// ---------------------------------------------------------------------------

const STDIN_FILENO: i32 = 0;
const STDOUT_FILENO: i32 = 1;
const TCSAFLUSH: i32 = 2;
const TIOCGWINSZ: u64 = 0x5413;
const SIGWINCH: i32 = 28;
const NCCS: usize = 32;

// Termios flag constants
const ECHO: u32 = 0o000010;
const ICANON: u32 = 0o000002;
const ISIG: u32 = 0o000001;
const IEXTEN: u32 = 0o100000;
const IXON: u32 = 0o002000;
const ICRNL: u32 = 0o000400;
const BRKINT: u32 = 0o000002;
const INPCK: u32 = 0o000020;
const ISTRIP: u32 = 0o000040;
const OPOST: u32 = 0o000001;
const CS8: u32 = 0o000060;

// sigaction constants
const SA_RESTART: u64 = 0x10000000;

#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; NCCS],
    _padding: [u8; 3],
    c_ispeed: u32,
    c_ospeed: u32,
}

impl Termios {
    fn zeroed() -> Self {
        // SAFETY: Termios is a plain data struct with no invariants.
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

// Linux x86-64 sigaction layout
#[repr(C)]
struct SigAction {
    sa_handler: extern "C" fn(i32),
    sa_flags: u64,
    sa_restorer: usize,
    sa_mask: [u64; 16], // sigset_t is 128 bytes on Linux x86-64
}

unsafe extern "C" {
    fn tcgetattr(fd: i32, termios: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, optional_actions: i32, termios: *const Termios) -> i32;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn sigaction(signum: i32, act: *const SigAction, oldact: *mut SigAction) -> i32;
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
}

// ---------------------------------------------------------------------------
// SIGWINCH handling
// ---------------------------------------------------------------------------

static RESIZED: AtomicBool = AtomicBool::new(false);

extern "C" fn sigwinch_handler(_sig: i32) {
    RESIZED.store(true, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------
// Color mode detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    TrueColor,
    Color256,
    Color16,
}

pub fn detect_color_mode() -> ColorMode {
    if let Ok(val) = std::env::var("COLORTERM") {
        let val = val.to_lowercase();
        if val == "truecolor" || val == "24bit" {
            return ColorMode::TrueColor;
        }
    }
    if let Ok(term) = std::env::var("TERM") {
        let term = term.to_lowercase();
        if term.contains("256color") {
            return ColorMode::Color256;
        }
    }
    ColorMode::Color16
}

// ---------------------------------------------------------------------------
// Terminal
// ---------------------------------------------------------------------------

pub struct Terminal {
    original: Termios,
    width: u16,
    height: u16,
}

impl Terminal {
    /// Create a new Terminal, enabling raw mode, alternate screen, mouse, and
    /// bracketed paste. The original terminal state is saved and will be
    /// restored when the Terminal is dropped.
    pub fn new() -> Result<Self, String> {
        let mut original = Termios::zeroed();

        // Save original terminal attributes
        if unsafe { tcgetattr(STDIN_FILENO, &mut original) } != 0 {
            return Err("Failed to get terminal attributes".into());
        }

        // Enable raw mode
        let mut raw = original;
        raw.c_iflag &= !(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
        raw.c_oflag &= !OPOST;
        raw.c_cflag |= CS8;
        raw.c_lflag &= !(ECHO | ICANON | IEXTEN | ISIG);
        // VMIN = 0, VTIME = 1 (100ms timeout for non-blocking reads)
        raw.c_cc[6] = 0; // VMIN
        raw.c_cc[5] = 1; // VTIME

        if unsafe { tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw) } != 0 {
            return Err("Failed to set raw mode".into());
        }

        // Query initial size
        let (width, height) = query_terminal_size()?;

        // Register SIGWINCH handler
        let sa = SigAction {
            sa_handler: sigwinch_handler,
            sa_flags: SA_RESTART,
            sa_restorer: 0,
            sa_mask: [0; 16],
        };
        if unsafe { sigaction(SIGWINCH, &sa, std::ptr::null_mut()) } != 0 {
            // Restore terminal before returning error
            unsafe { tcsetattr(STDIN_FILENO, TCSAFLUSH, &original) };
            return Err("Failed to register SIGWINCH handler".into());
        }

        // Enter alternate screen, enable mouse and bracketed paste, hide cursor
        write_all(b"\x1b[?1049h");
        enable_mouse();
        enable_bracketed_paste();

        Ok(Terminal {
            original,
            width,
            height,
        })
    }

    /// Return the current terminal size as (width, height), re-querying via ioctl.
    pub fn size(&mut self) -> (u16, u16) {
        if let Ok((w, h)) = query_terminal_size() {
            self.width = w;
            self.height = h;
        }
        (self.width, self.height)
    }

    /// Check if a SIGWINCH resize occurred. If so, refresh the cached size and
    /// return true.
    pub fn check_resize(&mut self) -> bool {
        if RESIZED.swap(false, Ordering::SeqCst) {
            self.size();
            true
        } else {
            false
        }
    }

    /// Read a single byte from stdin. Returns `None` on timeout / no data.
    pub fn read_byte(&self) -> Option<u8> {
        let mut buf: u8 = 0;
        let n = unsafe { read(STDIN_FILENO, &mut buf, 1) };
        if n == 1 { Some(buf) } else { None }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        disable_mouse();
        disable_bracketed_paste();
        show_cursor();
        write_all(b"\x1b[?1049l"); // leave alternate screen
        flush();
        unsafe {
            tcsetattr(STDIN_FILENO, TCSAFLUSH, &self.original);
        }
    }
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

pub fn write_all(buf: &[u8]) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(buf);
}

pub fn flush() {
    let _ = std::io::stdout().flush();
}

fn query_terminal_size() -> Result<(u16, u16), String> {
    let mut ws = Winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe { ioctl(STDOUT_FILENO, TIOCGWINSZ, &mut ws) } != 0 || ws.ws_col == 0 {
        return Err("Failed to query terminal size".into());
    }
    Ok((ws.ws_col, ws.ws_row))
}

// ---------------------------------------------------------------------------
// Escape sequence helpers
// ---------------------------------------------------------------------------

pub fn enable_mouse() {
    write_all(b"\x1b[?1000h\x1b[?1006h");
}

pub fn disable_mouse() {
    write_all(b"\x1b[?1006l\x1b[?1000l");
}

pub fn enable_bracketed_paste() {
    write_all(b"\x1b[?2004h");
}

pub fn disable_bracketed_paste() {
    write_all(b"\x1b[?2004l");
}

pub fn hide_cursor() {
    write_all(b"\x1b[?25l");
}

pub fn show_cursor() {
    write_all(b"\x1b[?25h");
}

pub fn move_cursor(row: u16, col: u16) {
    let seq = format!("\x1b[{};{}H", row, col);
    write_all(seq.as_bytes());
}

pub fn clear_screen() {
    write_all(b"\x1b[2J");
}
