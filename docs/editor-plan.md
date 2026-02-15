# Zelux — Zhell Editor LEXury

A modern console text editor written in pure Rust.

## Vision

A lightweight, modern terminal text editor written in pure Rust with **zero external dependencies**. Uses raw terminal mode via libc FFI (proven in the Minilux REPL). Designed for users who expect modern keybindings (Ctrl+C/V/X) out of the box — no modes, no learning curve.

Part of the **Z ecosystem**: Zenith, Zymbol, and now Zelux. Born from the Minilux REPL.

## Design Principles

1. **Zero dependencies** — only `std` + libc FFI (same approach as `minilux/src/repl.rs`)
2. **Modern keybindings by default** — Ctrl+C copy, Ctrl+V paste, Ctrl+X cut, Ctrl+S save, Ctrl+Z undo
3. **Instant startup** — sub-10ms cold start, single static binary
4. **Familiar UX** — behaves like a desktop editor trapped in a terminal
5. **UTF-8 native** — full Unicode support from day one
6. **Standard formats** — TextMate grammars and VS Code-compatible themes (no proprietary formats)

## Architecture

```text
src/
  main.rs              Entry point, argument parsing
  terminal.rs          Raw mode FFI, screen size, resize handling (SIGWINCH)
  input.rs             Key reading, escape sequence decoding, key mapping
  buffer.rs            Text buffer (gap buffer), line tracking
  editor.rs            Core editor state machine, command dispatch
  view.rs              Viewport, scrolling, line wrapping
  render.rs            Screen rendering, diff-based updates, colors
  cursor.rs            Cursor movement logic, multi-cursor support
  selection.rs         Text selection (char, line, block)
  clipboard.rs         Internal clipboard + OSC 52 terminal clipboard
  undo.rs              Undo/redo stack (operation-based)
  search.rs            Find, replace, incremental search
  status.rs            Status bar, command palette
  syntax/
    mod.rs             Syntax highlighting public API
    grammar.rs         TextMate grammar data structures
    json_parser.rs     Minimal JSON parser (zero deps)
    regex.rs           Regex engine subset for TextMate patterns
    tokenizer.rs       Line tokenizer using grammar rules
    scope.rs           Scope name handling and hierarchy matching
    theme.rs           Theme loader + scope-to-color resolution
  config.rs            Runtime configuration

grammars/              TextMate grammar files (.tmLanguage.json)
  rust.tmLanguage.json
  python.tmLanguage.json
  javascript.tmLanguage.json
  typescript.tmLanguage.json
  c.tmLanguage.json
  cpp.tmLanguage.json
  go.tmLanguage.json
  java.tmLanguage.json
  shellscript.tmLanguage.json
  markdown.tmLanguage.json
  json.tmLanguage.json
  toml.tmLanguage.json
  yaml.tmLanguage.json
  html.tmLanguage.json
  css.tmLanguage.json
  minilux.tmLanguage.json       (custom, we create this)

themes/                VS Code-compatible theme files (.json)
  zelux-dark.json
  zelux-light.json
```

## Module Details

### Phase 1 — Core Editing (~3 weeks)

#### `terminal.rs` — Terminal Abstraction

Extends the Minilux REPL's raw mode FFI:

```rust
pub struct Terminal {
    original: Termios,
    width: u16,
    height: u16,
}

impl Terminal {
    fn enable_raw_mode() -> Result<Self, String>;
    fn size() -> (u16, u16);           // ioctl TIOCGWINSZ
    fn enable_mouse() -> ();            // SGR mouse mode
    fn enable_bracketed_paste() -> ();  // \x1b[?2004h
}
```

FFI additions beyond REPL:

- `ioctl()` for terminal size (TIOCGWINSZ)
- `sigaction()` for SIGWINCH (terminal resize)
- Alternate screen buffer (`\x1b[?1049h/l`)

#### `input.rs` — Input Handling

Extends the REPL's key reader with modifier detection:

```rust
pub struct KeyEvent {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

pub enum Key {
    Char(char),
    Enter, Tab, Backspace, Delete, Escape,
    Up, Down, Left, Right,
    Home, End, PageUp, PageDown,
    F(u8),                    // F1-F12
}
```

New escape sequences to handle:

- `\x1b[1;5A` — Ctrl+Up (modifier encoding: `1;{modifier}`)
- `\x1b[1;2D` — Shift+Left
- `\x1b[?2004~` — Bracketed paste start/end
- SGR mouse events: `\x1b[<button;col;row;M/m`

#### `buffer.rs` — Text Storage

Gap buffer for simplicity and performance:

```rust
pub struct Buffer {
    data: Vec<u8>,           // UTF-8 bytes
    gap_start: usize,
    gap_end: usize,
    lines: Vec<usize>,       // byte offsets of line starts (cached)
    modified: bool,
    file_path: Option<PathBuf>,
}
```

Why gap buffer over rope:

- Simpler (~200 lines vs ~800 for a rope)
- Excellent performance for typical editing (sequential inserts)
- Good enough for files up to ~50MB
- No allocator overhead per node

#### `editor.rs` — Command Dispatch

```rust
pub struct Editor {
    buffers: Vec<Buffer>,
    active: usize,
    terminal: Terminal,
    clipboard: Clipboard,
    undo_stack: UndoStack,
    mode: Mode,              // Normal, Search, CommandPalette
    config: Config,
    running: bool,
}

impl Editor {
    pub fn run(&mut self) -> Result<(), String> {
        // Main loop: read input -> dispatch command -> render
    }
}
```

#### `cursor.rs` — Cursor Logic

```rust
pub struct Cursor {
    pub line: usize,         // 0-indexed line number
    pub col: usize,          // 0-indexed byte offset within line
    pub desired_col: usize,  // "sticky" column for vertical movement
}
```

### Phase 2 — Selection & Clipboard (~1 week)

#### `selection.rs`

```rust
pub struct Selection {
    pub anchor: Position,    // where selection started
    pub head: Position,      // where cursor currently is
    pub mode: SelectMode,    // Char, Line, Block
}
```

#### `clipboard.rs`

Two clipboard layers:

1. **Internal** — always works, stores `Vec<String>`
2. **Terminal** — OSC 52 escape sequence (`\x1b]52;c;{base64}\x07`) for system clipboard integration

### Phase 3 — Undo/Redo (~1 week)

#### `undo.rs`

Operation-based undo (not snapshot-based):

```rust
pub enum Operation {
    Insert { pos: usize, text: String },
    Delete { pos: usize, text: String },
    Replace { pos: usize, old: String, new: String },
}

pub struct UndoStack {
    undo: Vec<Vec<Operation>>,    // grouped by transaction
    redo: Vec<Vec<Operation>>,
    current_group: Vec<Operation>,
}
```

Grouping rules:

- Sequential character inserts at adjacent positions = one group
- Paste = one group
- Cut = one group
- 500ms pause = start new group

### Phase 4 — Search & Replace (~1 week)

#### `search.rs`

```rust
pub struct Search {
    pub pattern: String,
    pub matches: Vec<(usize, usize)>,    // (byte_start, byte_end)
    pub current: Option<usize>,
    pub case_sensitive: bool,
    pub regex: bool,                      // basic regex, no crate needed
}
```

Incremental search: highlights update as the user types.

### Phase 5 — Rendering Engine (~2 weeks)

#### `render.rs`

Diff-based rendering to minimize terminal output:

```rust
pub struct Screen {
    cells: Vec<Vec<Cell>>,        // current screen state
    prev_cells: Vec<Vec<Cell>>,   // previous frame
}

pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
}
```

Only emit ANSI sequences for cells that changed between frames. Use `\x1b[{row};{col}H` for cursor positioning.

#### `status.rs`

Two-line status area at bottom:

- **Line 1**: filename | modified indicator | cursor position | encoding | line ending
- **Line 2**: contextual messages, search input, command palette

### Phase 6 — Syntax Highlighting via TextMate Grammars (~5 weeks)

Syntax highlighting uses **TextMate grammars** (`.tmLanguage.json`) — the industry standard format used by VS Code, Sublime Text, Atom, and many others. This gives us access to **thousands of existing language definitions** maintained by their respective communities under open-source licenses (typically MIT).

#### Why TextMate Grammars

| Option                         | Pros                                                                                                    | Cons                                                  |
| ------------------------------ | ------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| **TextMate (.tmLanguage.json)**| Industry standard, 600+ languages available, used by VS Code, MIT-licensed grammars, well-documented    | Regex-heavy, complex to implement fully               |
| Tree-sitter                    | Accurate parsing, incremental                                                                           | Requires compiling C grammars, massive dependency     |
| Vim syntax                     | Large collection                                                                                        | Vimscript-dependent, hard to parse                    |
| Kate XML                       | Good quality                                                                                            | XML parsing overhead, fewer languages                 |
| Custom format                  | Simple to implement                                                                                     | Zero ecosystem, reinventing the wheel                 |

TextMate is the clear winner: maximum reuse, zero licensing issues, huge ecosystem.

#### TextMate Grammar Format Overview

TextMate grammars are JSON files with regex-based pattern matching. Key structure:

```json
{
  "name": "Rust",
  "scopeName": "source.rust",
  "fileTypes": ["rs"],
  "patterns": [
    { "include": "#comments" },
    { "include": "#strings" },
    { "include": "#keywords" }
  ],
  "repository": {
    "comments": {
      "patterns": [
        { "name": "comment.line.double-slash.rust",
          "match": "//.*$" },
        { "name": "comment.block.rust",
          "begin": "/\\*",
          "end": "\\*/",
          "patterns": [{ "include": "#comments" }] }
      ]
    },
    "keywords": {
      "patterns": [
        { "name": "keyword.control.rust",
          "match": "\\b(if|else|while|for|loop|match|return|break|continue)\\b" }
      ]
    },
    "strings": {
      "patterns": [
        { "name": "string.quoted.double.rust",
          "begin": "\"",
          "end": "\"",
          "patterns": [
            { "name": "constant.character.escape.rust",
              "match": "\\\\." }
          ] }
      ]
    }
  }
}
```

Scope names follow a hierarchy (e.g., `keyword.control.rust`, `string.quoted.double`, `comment.block`) that maps to theme colors.

#### Implementation Architecture

```text
src/
  syntax/
    mod.rs              Public API: load grammar, tokenize line
    grammar.rs          TextMate grammar data structures
    json_parser.rs      Minimal JSON parser (no external deps)
    regex.rs            Regex engine (subset needed for TextMate patterns)
    tokenizer.rs        Line tokenizer using grammar rules
    scope.rs            Scope name handling and hierarchy matching
    theme.rs            Theme loading + scope-to-color resolution
```

#### `json_parser.rs` — Minimal JSON Parser (~300 lines)

Since we have zero external dependencies, we need a minimal JSON parser. It only needs to handle the subset used by `.tmLanguage.json` files:

```rust
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),  // preserves order
}

impl JsonValue {
    pub fn parse(input: &str) -> Result<JsonValue, String>;
    pub fn as_str(&self) -> Option<&str>;
    pub fn as_object(&self) -> Option<&[(String, JsonValue)]>;
    pub fn as_array(&self) -> Option<&[JsonValue]>;
    pub fn get(&self, key: &str) -> Option<&JsonValue>;
}
```

This is well-scoped: JSON is a simple grammar, and we only need read-only parsing.

#### `regex.rs` — Regex Engine Subset (~500 lines)

TextMate grammars use Oniguruma-style regexes. We implement a subset sufficient for >95% of real-world grammar patterns:

**Supported features:**

- Literals, escapes (`\n`, `\t`, `\\`, `\.`)
- Character classes: `[a-z]`, `[^0-9]`, `\w`, `\d`, `\s`, `\b`
- Quantifiers: `*`, `+`, `?`, `{n}`, `{n,m}`
- Alternation: `a|b`
- Groups: `(...)`, `(?:...)` (non-capturing)
- Anchors: `^`, `$`, `\b`
- Lookahead: `(?=...)`, `(?!...)` (needed by some grammars)
- Backreferences: `\1` (needed for heredocs, matched delimiters)

**Not supported (rare in practice):**

- Lookbehind (`(?<=...)`) — very few grammars use this
- Unicode categories (`\p{L}`) — fallback to `\w`
- Recursive patterns — extremely rare

Strategy: compile regex to a simple NFA/bytecode, execute with backtracking. Performance is acceptable because we only match one line at a time against a small number of patterns.

#### `grammar.rs` — Data Structures

```rust
pub struct Grammar {
    pub name: String,
    pub scope_name: String,
    pub file_types: Vec<String>,
    pub patterns: Vec<Pattern>,
    pub repository: HashMap<String, PatternGroup>,
}

pub enum Pattern {
    /// Single-line match
    Match {
        name: Option<String>,         // scope name
        match_re: CompiledRegex,
        captures: HashMap<usize, String>,  // capture group -> scope
    },
    /// Multi-line region (begin/end)
    Region {
        name: Option<String>,
        begin: CompiledRegex,
        end: CompiledRegex,
        begin_captures: HashMap<usize, String>,
        end_captures: HashMap<usize, String>,
        content_name: Option<String>,
        patterns: Vec<Pattern>,       // patterns active inside the region
    },
    /// Reference to a repository rule
    Include(String),
}
```

#### `tokenizer.rs` — Line Tokenizer

```rust
pub struct ScopeToken {
    pub start: usize,        // byte offset in line
    pub end: usize,
    pub scope: String,       // e.g. "keyword.control.rust"
}

pub struct LineState {
    pub scope_stack: Vec<ActiveRegion>,   // for tracking open begin/end regions
}

pub struct Tokenizer {
    grammar: Grammar,
}

impl Tokenizer {
    /// Tokenize a single line given the state from the previous line
    pub fn tokenize_line(&self, line: &str, state: &LineState) -> (Vec<ScopeToken>, LineState);
}
```

The `LineState` is carried between lines to track open multi-line regions (strings, comments, etc.). This allows re-tokenizing only changed lines + propagating state changes downward.

#### Grammar File Search Paths

1. `~/.config/zelux/grammars/` — user-installed grammars (highest priority)
2. `./grammars/` — project-local grammars
3. Built-in defaults embedded at compile time via `include_str!`

The editor ships with embedded grammars for the most common languages. Users can download any VS Code grammar file and drop it in their config directory.

#### Where to Get Grammars

All MIT or similar permissive licenses:

| Language       | Source Repository                      |
| -------------- | -------------------------------------- |
| Rust           | `ArtifexSoftware/syntect` (Apache 2.0)|
| C/C++          | `ArtifexSoftware/syntect` (Apache 2.0)|
| Python         | `MagicStack/MagicPython` (MIT)         |
| JavaScript     | `ArtifexSoftware/syntect` (Apache 2.0)|
| TypeScript     | `ArtifexSoftware/syntect` (Apache 2.0)|
| Go             | `ArtifexSoftware/syntect` (Apache 2.0)|
| Java           | `ArtifexSoftware/syntect` (Apache 2.0)|
| Shell/Bash     | `ArtifexSoftware/syntect` (Apache 2.0)|
| Markdown       | `ArtifexSoftware/syntect` (Apache 2.0)|
| JSON/TOML/YAML | `ArtifexSoftware/syntect` (Apache 2.0)|
| HTML/CSS       | `ArtifexSoftware/syntect` (Apache 2.0)|
| Minilux        | Custom (we create this one)            |

Note: we only take the `.tmLanguage.json` data files, not the syntect Rust crate itself. Our engine is a clean-room implementation.

#### Theme System — TextMate-Compatible Scopes

Themes map TextMate scope selectors to colors. VS Code-compatible format:

```json
{
  "name": "Zelux Dark",
  "type": "dark",
  "colors": {
    "editor.background": "#1e1e2e",
    "editor.foreground": "#cdd6f4",
    "editorLineNumber.foreground": "#6c7086",
    "editor.selectionBackground": "#45475a",
    "editorCursor.foreground": "#f5e0dc",
    "statusBar.background": "#181825",
    "statusBar.foreground": "#cdd6f4"
  },
  "tokenColors": [
    { "scope": "comment", "settings": { "foreground": "#6c7086", "fontStyle": "italic" } },
    { "scope": "string", "settings": { "foreground": "#a6e3a1" } },
    { "scope": "keyword", "settings": { "foreground": "#cba6f7", "fontStyle": "bold" } },
    { "scope": "constant.numeric", "settings": { "foreground": "#fab387" } },
    { "scope": "constant.language", "settings": { "foreground": "#fab387" } },
    { "scope": "storage.type", "settings": { "foreground": "#89b4fa" } },
    { "scope": "entity.name.function", "settings": { "foreground": "#89b4fa" } },
    { "scope": "entity.name.type", "settings": { "foreground": "#f9e2af" } },
    { "scope": "variable", "settings": { "foreground": "#cdd6f4" } },
    { "scope": "support.function", "settings": { "foreground": "#f5c2e7" } },
    { "scope": "invalid", "settings": { "foreground": "#f38ba8", "fontStyle": "bold underline" } }
  ]
}
```

This format is **directly compatible with VS Code themes** — users can adapt existing themes with minimal changes.

Scope matching uses TextMate's hierarchy rules:

- `keyword` matches `keyword.control.rust`, `keyword.operator.rust`, etc.
- `keyword.control` matches `keyword.control.rust` but not `keyword.operator.rust`
- More specific matches take priority

Color fallback strategy for limited terminals:

- 24-bit (`COLORTERM=truecolor`) — use exact hex colors
- 256 colors (`256color` in `$TERM`) — map to nearest xterm-256 palette entry
- 16 colors — map to ANSI base colors (keyword=magenta, string=green, etc.)

#### Highlighting Strategy

1. On file open: detect language from file extension, load matching `Grammar`
2. Tokenize each visible line using `Tokenizer::tokenize_line()`, carrying `LineState` between lines
3. On edit: re-tokenize from the edited line downward until `LineState` matches the previously cached state (usually stops after 1-3 lines)
4. Tokenization runs synchronously for visible lines only (lazy for off-screen content)

## Keybinding Map

### File Operations

| Key              | Action                          |
| ---------------- | ------------------------------- |
| `Ctrl+S`         | Save                            |
| `Ctrl+Shift+S`   | Save as                         |
| `Ctrl+O`         | Open file                       |
| `Ctrl+N`         | New buffer                      |
| `Ctrl+W`         | Close buffer                    |
| `Ctrl+Q`         | Quit (confirm if unsaved)       |

### Editing

| Key              | Action                          |
| ---------------- | ------------------------------- |
| `Ctrl+C`         | Copy selection (or current line if no selection) |
| `Ctrl+X`         | Cut selection (or current line) |
| `Ctrl+V`         | Paste                           |
| `Ctrl+Z`         | Undo                            |
| `Ctrl+Y`         | Redo                            |
| `Ctrl+D`         | Duplicate line                  |
| `Ctrl+Shift+K`   | Delete line                     |
| `Tab`            | Indent (or insert tab)          |
| `Shift+Tab`      | Unindent                        |
| `Ctrl+/`         | Toggle line comment             |
| `Enter`          | New line with auto-indent       |

### Navigation

| Key              | Action                          |
| ---------------- | ------------------------------- |
| `Ctrl+G`         | Go to line                      |
| `Ctrl+Home`      | Go to file start                |
| `Ctrl+End`       | Go to file end                  |
| `Ctrl+Left/Right`| Word jump                       |
| `PageUp/PageDown`| Page scroll                     |
| `Home`           | Start of line (smart: toggle between indent and column 0) |
| `End`            | End of line                     |

### Selection

| Key                      | Action                  |
| ------------------------ | ----------------------- |
| `Shift+Arrow`            | Extend selection        |
| `Ctrl+Shift+Left/Right`  | Select word             |
| `Shift+Home/End`         | Select to line start/end|
| `Ctrl+A`                 | Select all              |
| `Ctrl+L`                 | Select line             |

### Search

| Key                  | Action                  |
| -------------------- | ----------------------- |
| `Ctrl+F`             | Find                    |
| `Ctrl+H`             | Find and replace        |
| `F3` / `Shift+F3`    | Next / previous match   |
| `Escape`             | Close search            |

### Multi-buffer

| Key              | Action          |
| ---------------- | --------------- |
| `Ctrl+Tab`       | Next buffer     |
| `Ctrl+Shift+Tab` | Previous buffer |

## Terminal Compatibility

Target terminals:

- Linux: xterm, gnome-terminal, konsole, alacritty, kitty, foot, tmux, screen
- macOS: Terminal.app, iTerm2
- Windows: Windows Terminal, mintty (WSL)

Feature detection strategy:

- Terminal size: `ioctl(TIOCGWINSZ)` with `SIGWINCH` for resize
- Color support: check `$TERM` and `$COLORTERM` env vars
  - `COLORTERM=truecolor` — 24-bit color
  - `256color` in `$TERM` — 256 colors
  - Fallback — 16 colors
- Clipboard: try OSC 52, degrade gracefully to internal-only
- Mouse: enable SGR mode, disable on exit
- Bracketed paste: enable `\x1b[?2004h`, disable on exit

## Performance Targets

| Metric                        | Target  |
| ----------------------------- | ------- |
| Startup time                  | < 10ms  |
| Keypress-to-screen latency    | < 5ms   |
| Open 1MB file                 | < 50ms  |
| Open 50MB file                | < 500ms |
| Memory for 1MB file           | < 5MB   |
| Binary size                   | < 1MB   |

## Build & Distribution

```sh
cargo build --release            # Single static binary
strip target/release/zelux       # ~500KB
```

Works standalone with embedded defaults. Optional config at `~/.config/zelux/`:

```text
~/.config/zelux/
  config.json           General settings (tab size, line numbers, etc.)
  theme.json            Active color theme (VS Code-compatible)
  grammars/             User grammars (override built-ins)
    my-language.tmLanguage.json
```

## Timeline Estimate

This is a substantial project. Realistic timeline for a working MVP:

| Phase    | Scope                                                    | Estimate  |
| -------- | -------------------------------------------------------- | --------- |
| Phase 1  | Core editing (terminal, input, buffer, cursor, render)   | ~3 weeks  |
| Phase 2  | Selection & clipboard                                    | ~1 week   |
| Phase 3  | Undo/redo                                                | ~1 week   |
| Phase 4  | Search & replace                                         | ~1 week   |
| Phase 5  | Rendering engine (diff-based, colors)                    | ~2 weeks  |
| Phase 6a | JSON parser + regex engine (foundations)                  | ~2 weeks  |
| Phase 6b | TextMate grammar loader + tokenizer                      | ~2 weeks  |
| Phase 6c | Theme system + scope-to-color resolution                 | ~1 week   |
| Phase 7  | Polish, edge cases, terminal compat testing              | ~2 weeks  |
| **Total MVP** |                                                     | **~15 weeks** |

Post-MVP phases (each ~2-4 weeks):

| Phase    | Feature                                                  |
| -------- | -------------------------------------------------------- |
| Phase 8  | Multi-cursor editing (Ctrl+D select next occurrence)     |
| Phase 9  | File tree sidebar (split terminal view)                  |
| Phase 10 | Soft word wrap for prose editing                         |
| Phase 11 | Session restore (remember open files and cursor positions) |
| Phase 12 | Integrated terminal (split pane with shell)              |
| Phase 13 | LSP client (completions, diagnostics, go-to-definition)  |
| Phase 14 | Plugin system (Minilux scripting)                        |
| Phase 15 | Diff view (side-by-side with git integration)            |
| Phase 16 | Minimap (code overview sidebar)                          |

## Community & Extensibility

Zelux uses only standard, open formats:

- **Syntax definitions**: TextMate `.tmLanguage.json` — thousands available from VS Code extensions, Sublime Text packages, and the syntect project. All open-source (MIT/Apache 2.0). Any grammar that works in VS Code works in Zelux.
- **Themes**: VS Code-compatible `tokenColors` JSON — users can port any VS Code theme directly.
- **Configuration**: JSON — universally understood, no custom parsers needed.

Contributing a new language = downloading a `.tmLanguage.json` file. No compilation required.

## Inspiration & References

| Editor   | What to take                        | What to avoid                            |
| -------- | ----------------------------------- | ---------------------------------------- |
| nano     | Simplicity, discoverability         | Limited features, no syntax highlighting |
| micro    | Modern keybindings, plugin system   | Go dependency, startup overhead          |
| vis      | Structural regex, composability     | Steep learning curve                     |
| kakoune  | Selection-first model               | Non-standard keybindings                 |
| helix    | Tree-sitter, multiple cursors       | Large binary, complex dependencies       |

## Project Name

**Zelux** — **Z**hell **E**ditor **L**EX**u**ry. Part of the Z ecosystem (Zenith, Zymbol). The "lux" suffix honors Minilux, where the idea was born.

```text
          _
 ______  | | _   ___  __
|_  /__ \| || | | \ \/ /
 / / __) | || |_| |>  <
/___\___/|_| \__,_/_/\_\
  console editor
```
