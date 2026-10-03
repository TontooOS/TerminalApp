# Terminal

The Terminal window and the terminal emulator behind it: a TontooUI
window with a macOS-style `Titlebar`, a grid drawn into the Vello scene
and a VT parser that drives the grid.

## Window

```rust
fn main() {
    lang::init();
    let app = app::TerminalApp::new();
    run(&lang::t("app.title"), config::WINDOW_WIDTH, config::WINDOW_HEIGHT, app)
}
```

- The window is `1170` x `600` logical px (`config::WINDOW_WIDTH` and
  `config::WINDOW_HEIGHT`); the content viewport sits inside the TontooUI
  frame, so the grid area is a little smaller.
- `TerminalApp` implements `TontooUI::renderer::window::App`. `draw`
  polls the theme, measures the font, syncs the grid with the PTY, draws
  the `Titlebar` and then the grid.
- `background` returns the theme background, so the body is a flat fill:
  no `transparent_body`, no `wants_backdrop`, no blur.
- `window_title` mirrors the visible title, so the compositor and the
  task switcher see the running program too.
- `drag_region` is the title bar minus the title text: dragging by the
  bar moves the window, clicking the title opens the folder in Finder.
- Traffic lights map to `WindowCommand::Close`, `Minimize` and
  `ToggleMaximize`; a shell that exits sends `Close`.

## Titlebar

`Titlebar::height(TitlebarHeight::Mac)` gives the 44 px macOS bar with
traffic lights. The palette comes from the `ThemeWatcher` each frame
(`palette.titlebar_bg`, `titlebar_text`, `divider`).

| Title source | Value |
|---|---|
| OSC 0 / 1 / 2 | program title, wins over everything |
| OSC 7 | current folder, percent decoded to a path |
| Fallback | `lang::t("terminal.untitled")` |

## Grid

```rust
pub struct GridRenderer {
    font_size: f32,
    family: String,
    cell_width: f32,
    row_height: f32,
    measured_scale: f32,
    layouts: HashMap<LayoutKey, CTFrame>,
}
```

- `measure` lays out 40 `M` glyphs once per window scale and derives
  `cell_width` from the advance, so the grid follows whatever monospace
  face resolves.
- Grid text is always laid out with CoreText's **monospace generic
  family** (`layout_mono`, a `RichSpan` with `monospace`), never by
  family name: CoreText pushes each named family as one quoted entry,
  so a list like `"SF Mono", monospace` resolves to a single missing
  family and silently falls back to the *proportional* system font. That
  fallback keeps a uniform advance only by accident, so a "M" measured
  the cell width while the text drew at its own advance and the cursor
  drifted to the right of the line.
- `grid_size(area)` returns `(cols, rows)`; the app resizes the screen
  and the PTY when it changes.
- `draw` records one fill for the body, then for every visible row: one
  fill per style run (cell background, selection veil, inverse) and one
  CoreText layout per run. Runs are cached in `layouts` (keyed by text,
  color and boldness, at most `MAX_CACHED_RUNS` entries), so a redraw
  only re-records the scene.
- Attributes are honored: bold (weight 700), dim (60% brightness),
  underline, strikethrough, blink, inverse (colors swapped), hidden
  (text skipped) and inverse video.
- The cursor follows `DECSCUSR`: a filled block that inverts the glyph,
  an underline bar or a bar cursor. An unfocused window draws an
  outline block instead of the fill.
- The cursor is static, never blinking: a blinking block flipped the
  glyph under it between the text and the background color every cycle,
  which read as flickering text. Only SGR 5 and 6 text blinks
  (`config::BLINK_SECONDS`), because a program asked for it.
- `selection` is addressed by virtual line index and column, so it
  survives scrolling and new output.
- The background fill splits a style run where the selection starts or
  ends (`segment_end`). Without that split a selection inside a long
  run, which is the normal case for a whole prompt line, inherited the
  unselected state of the run start and was never painted.

## Colors

`theme` resolves every cell color against the active mode.

| Token | Dark | Light |
|---|---|---|
| Background | `#1b2022` | `#ffffff` |
| Text | `#d8d9d9` | `#272727` |
| Title bar | `ThemeWatcher` palette | `ThemeWatcher` palette |
| Selection | `rgba(74, 90, 98, 153)` | `rgba(191, 212, 242, 153)` |

- Program output uses the classic 16 color ANSI palette
  (`config::PALETTE`); black, white and bright black are lifted or
  dimmed per mode so they stay readable.
- `38;5;n` and `38;2;r;g;b` (plus the background forms) resolve through
  the xterm 256 color cube: palette `0..15`, `6x6x6` cube `16..231`,
  gray ramp `232..255`.
- OSC 10 and OSC 11 are answered with `rgb:rrrr/gggg/bbbb` built from
  the token, so a shell sees the configured colors.

## VT Parser

`Parser::feed(&[u8], &mut Screen, &mut Vec<u8>, f64)` runs the state
machine. Bytes may be split across calls; a sequence spanning two reads
is buffered.

| Group | Supported |
|---|---|
| C0 | BEL, BS, HT, LF, VT, FF, CR, SO, SI, CAN, SUB |
| Cursor | CUU, CUD, CUF, CUB, CNL, CPL, CHA, CUP, HVP, CHT, CBT, VPA, HPA |
| Erase | ED (0, 1, 2, 3), EL (0, 1, 2), ECH |
| Edit | ICH, DCH, IL, DL, SU, SD, REP |
| Scroll | DECSTBM region, index, reverse index, full reset (RIS) |
| SGR | 0 to 9, 21 to 29, 30 to 37, 38, 39, 40 to 47, 48, 49, 90 to 97, 100 to 107 |
| Modes | SM/RM (4, 20), DECSET/DECRST (1, 5, 6, 7, 8, 9, 25, 47, 1047, 1048, 1049, 1000, 1002, 1003, 1004, 1006, 2004) |
| Charsets | `ESC ( x` / `ESC ) x`, DEC Special Graphics via SI/SO |
| Reports | DSR 5 and 6, DA 1 (`CSI ? 1 ; 2 c`), OSC 10 and 11 |
| OSC | 0, 1, 2 (title), 7 (folder), 8 (hyperlink), 52 (clipboard) |
| Terminal | DECSC/DECRC, DECALN (`ESC # 8`), tab stops (HTS, TBC) |

- `Screen::modes` holds the xterm defaults: autowrap on, cursor visible
  and blinking, insert mode, newline mode, origin mode, application
  cursor keys, keypad mode, bracketed paste, mouse modes and the
  alternate screen flag.
- The alternate screen (`47`, `1047`, `1049`) swaps the grid and saves
  the cursor; the scrollback belongs to the primary screen only.
- The scrollback is a `VecDeque` capped at `config::SCROLLBACK_LINES`
  (10000). Lines that scroll out of the region enter it; the visible
  window is `scroll_offset` lines above the bottom, so a scrolled view
  stays pinned while output arrives.

## Input

| Input | Result |
|---|---|
| Printable key | UTF-8 bytes, `ESC` prefixed with Alt |
| Ctrl+letter | control byte (`Ctrl+C` sends `0x03`) |
| Enter, KeypadEnter | `CR` |
| Tab / Shift+Tab | `HT` / `CSI Z` |
| Backspace | `DEL` (`0x7f`) |
| Arrows, Home, End | `CSI A..D`, `CSI H`, `CSI F`, `SS3` in application mode |
| Insert, Delete, PageUp, PageDown | `CSI 2 ~`, `3 ~`, `5 ~`, `6 ~` |
| F1 to F4 | `SS3 P..S` |
| F5 to F12 | `CSI 15 ~`, `17 ~`, `18 ~`, `19 ~`, `20 ~`, `21 ~`, `23 ~`, `24 ~` |
| Modified keys | `CSI 1 ; mod <final>` with `mod` = bits + 1 (shift 1, alt 2, ctrl 4, meta 8) |
| Keypad | digits and operators as `SS3 p..y` in application keypad mode |
| Wheel | three lines per notch through the scrollback |

- `App::raw_key` supplies every key with the full modifier state, so
  Ctrl chords, Tab and the function keys reach the shell unchanged.
- Clipboard shortcuts resolve in `shortcut(press)`:

  | Shortcut | Chords |
  |---|---|
  | Copy | `Super+C`, `Ctrl+Shift+C`, `Ctrl+Insert` |
  | Paste | `Super+V`, `Ctrl+Shift+V`, `Shift+Insert` |
  | Select all | `Super+A`, `Ctrl+Shift+A` |

  `Ctrl+C` and `Ctrl+V` deliberately stay shell input (`SIGINT` and
  "quote next"); stealing them would break every running program.
- Middle click and right click paste; left drag selects, double click
  selects a word, triple click a line, Shift+click extends.
- Shift with arrows, Home or End extends the selection instead of
  sending the key.
- Mouse reports follow xterm: SGR (`1006`) when enabled, X10 otherwise.
  Mode 1000 reports press and release, 1002 adds drag motion, 1003 all
  motion, 1004 focus changes.
- Pasted text is wrapped in `ESC [ 200 ~` and `ESC [ 201 ~` when mode
  2004 is on; newlines become `CR`.
- An OSC 52 write from a program goes to the system clipboard.

## Shell

```rust
pub fn spawn(
    shell: &str,
    args: &[&str],
    cwd: &str,
    cols: u16,
    rows: u16,
    env: &[(String, String)],
) -> Result<Pty, PtyError>
```

- `openpty` plus `fork`, `setsid` and `TIOCSCTTY`: the shell owns the
  terminal, so `Ctrl+C` reaches its foreground process group only.
- Everything the child needs is built before the `fork`, because only
  async-signal-safe work may follow it in a multithreaded process: the
  child re-points its standard streams, changes directory and `execve`s.
  A missing shell exits with 127.
- The master fd is `FD_CLOEXEC` and the reader duplicates it with
  `F_DUPFD_CLOEXEC`, so no other child of the window process can keep
  the terminal open.
- A reader thread does the blocking reads and sends `Output::Data`
  chunks over a channel; the render loop drains it per frame, so program
  output never blocks the UI thread. On EOF the thread waits for the
  child and reports its exit status.
- `resize` sets `TIOCSWINSZ`, which delivers `SIGWINCH` to the shell.
- Dropping the `Pty` closes the master, which hangs up the shell.

## Prompt

`prompt.rs` points zsh at a generated `$ZDOTDIR`
(`$XDG_RUNTIME_DIR/tontoo-terminal`, else `/tmp/tontoo-terminal-<uid>`)
whose `.zshenv` and `.zshrc` source the real system and user files first
and `Resources/tontoo-prompt.zsh` last. That file sets
`PROMPT='%F{2}%n@%m %~ %#%f '` (green, `%` for users and `#` for root)
and reports the folder with OSC 7. Themes may rebuild `PROMPT`, so the
format is enforced again in a `precmd` hook.

## Localization

`lang.rs` loads `lang/en_us.json` and `lang/de_de.json` through the
TontooOS Accessibility framework (`LangFile::from_file`,
`LangStore::init`, `LangStore::instance().t`). The locale comes from
`LANGUAGE`, `LC_ALL`, `LANG` or `/etc/locale.conf`. A missing key
returns the key itself.

## Environment

| Variable | Effect |
|---|---|
| `TONTOO_TERMINAL_SHELL` | Shell binary, wins over `SHELL` |
| `SHELL` | Used when `TONTOO_TERMINAL_SHELL` is unset |
| `TONTOO_TERMINAL_FONT_SIZE` | Grid font size in logical px, clamped to 6..72 |
| `TERM` | Set to `xterm-256color` when unset, plus `COLORTERM=truecolor` |
| `ZDOTDIR` | Points zsh at the generated prompt directory |
| `TONTOO_REALHOME` | Real home, read by the generated `.zshrc` |

## App Icon

The icon is a Tontoo `.tico` container at `Resources/icon.tico`, built
from `Resources/app_icon.png` with the CoreIcon example
(`examples/tico_from_png`, the same route the Xcode app uses):

```bash
cd ../../TontooLibs/CoreIcon/examples/tico_from_png
cargo run -- \
  ../../../../TontooMicroApps/Terminal/Resources/app_icon.png \
  Terminal \
  ../../../../TontooMicroApps/Terminal/Resources/icon.tico \
  /tmp/terminal-icon-preview.png
```

| Argument | Value |
|---|---|
| Input | `Resources/app_icon.png`, 1024x1024 (the CoreIcon canvas) |
| Name | `Terminal`, written into the container manifest |
| Output | `Resources/icon.tico`, about 3.7 MB |
| Preview | optional PNG; the default render there, a tinted render beside it |

The example stores the artwork as exactly one image layer
(`layer/00.tlyr`) over a transparent background and flags it
non-recolorable, so the container keeps the artwork colors while
`TicoIcon::render` adds the Apple icon finish (squircle, glass depth)
on top. It finishes with a round-trip check: the written container is
loaded again and rendered, and the layer count and render size are
printed.

`tontoo.proj` points at the container:

```json
{
  "bundle_id": "com.tontoo.terminal",
  "name": "Terminal",
  "version": "27.0.0",
  "icon": "Resources/icon.tico"
}
```

`app_icon.png` stays next to it as the source for a regeneration.

## Cross References

- [MAIN.md](MAIN.md) – project overview and changelog
- [RULE.md](RULE.md) – wiki design system and repo rules
- TontooUI `Titlebar` – the macOS decoration bar drawn here
- TontooUI `App` trait – `raw_key`, `mouse_button` and `window_title`
  are the hooks this app relies on
- Accessibility framework – `lang/` loading and `t()` lookups
- Foundation `NSPasteBoard` – clipboard for selection and paste