# Terminal

TontooOS terminal emulator basis (v1): one VTE terminal per window, no
tabs or splits. The UIKit decoration bar shows the live title, the
content area is the terminal itself (keyboard input, selection and
scrollback are native VTE behavior, macOS style needs no extra input
row). Colors cover the full ANSI palette, the bell is audible plus a
short visual flash, and the default shell is `zsh`.

## Layout

From top to bottom the window contains:

1. Decoration bar (`WindowType::Mac`, 44px, traffic lights plus centered title)
2. VTE terminal (fills the remaining area, transparent background)

```rust
let mut app = App::with_delegate(lang::t("app.title"), 1170, 600, TerminalDelegate);
app.set_window_type(WindowType::Mac);
app.set_window_transparency(0.85);
app.set_window_blur(20.0);
app.auto_color_scheme();
app.run();
```

## Terminal view

`src/terminal.rs::TerminalRoot` implements `UIKit::Widget`. It returns
a shared `vte4::Terminal` so UIKit rebuilds (resize, fullscreen toggle,
theme change) reparent the same widget instead of killing the shell
session.

| Behavior | Implementation |
|---|---|
| Shared instance | `thread_local TERMINAL: RefCell<Option<vte4::Terminal>>` |
| First use | `Terminal::new()`, `configure()`, `connect_signals()`, `spawn_shell()` |
| Rebuilds | `apply_theme()` plus `refresh_title()`, same widget reparented |
| Focus | `grab_focus()` on every `to_gtk()` so typing works immediately |

### `shared_terminal`

```rust
fn shared_terminal() -> vte4::Terminal
```

Returns the shared terminal, creating and spawning it on first use.
Recreates nothing on later calls; reapplies the theme so live
Dark/Light switches update the VTE colors.

## Shell

Default shell is `zsh`, resolved by `src/config.rs::resolve_shell()`.

| Source | Rule |
|---|---|
| `TONTOO_TERMINAL_SHELL` | Used when set to an existing binary |
| `SHELL` | Used when the override above is missing |
| `/bin/zsh` | Default when neither env var points to a binary |
| `/bin/bash`, `/bin/sh` | Fallback when `zsh` is not installed |

```rust
pub fn resolve_shell() -> String
```

Returns the shell binary path. Never returns an empty string; falls
back to `/bin/bash` then `/bin/sh` so the window always opens.

The shell starts in the current user home (`src/config.rs::home_dir()`:
`$HOME` when it is an existing directory, else the passwd entry for the
current uid, so root lands in `/root`; last resort `/tmp`, never `/`).

### `shell_basename`

```rust
pub fn shell_basename(shell: &str) -> &str
```

Returns the file name of a shell path (`/bin/zsh` becomes `zsh`).
Used for fallback titles.

Child exit (`exit` or shell EOF) closes the window
(`connect_child_exited`), like macOS Terminal. A failed spawn closes the
window as well instead of showing a dead terminal.

## Title

The bar shows the running program when it sets an OSC 0/1/2 title
(e.g. `nvim`, `htop`), else the current path from the VTE
`current-directory-uri`, else `app.title`.

| Signal | State | Effect |
|---|---|---|
| `window-title-changed` | `OSC_TITLE` | Bar updates immediately via `refresh_title()` |
| `current-directory-uri-changed` | `CWD_URI` | Bar updates only when no OSC title is set |
| `child-exited` | window | Closes the window (`exit` quits like macOS Terminal) |

```rust
fn display_title() -> String
```

Returns the OSC title when non-empty, else the `file://` URI converted
to a filesystem path, else the localized `app.title`. Returns the key
itself when localization is missing, so the bar never renders empty.

The title is pushed to both the GTK window (`window.set_title`, used by
the compositor and task switcher) and the UIKit decoration label (the
`uikit-titlebar-title` label found by tree walk).

Clicking the title opens the current shell folder in the file manager
(pointer cursor plus `title.open_in_finder` tooltip). The click handler
prefers the native `finder` binary and falls back to `xdg-open`; the
path comes from `current-directory-uri` with the passwd-resolved home
as fallback. The click controller is attached once per label, so signal
refreshes between rebuilds never stack handlers.

## Colors

Full ANSI color support comes from VTE plus a 16-entry macOS-like
palette (`src/config.rs::PALETTE`). The foreground and background
follow the live scheme on every rebuild.

| Token | Dark | Light |
|---|---|---|
| Background | `#1d1d1d` | `#ececec` |
| Primary text | `#F5F5F7` | `#1E1E1E` |
| Prompt `[user@machine folder]` | green (ANSI 2) | green (ANSI 2) |
| Palette | 16 ANSI colors | 16 ANSI colors |

The VTE background is transparent (`set_clear_background(false)` plus
`vte-terminal { background-color: transparent; }`), so the UIKit window
background (Dark `#1d1d1d` / Light `#ececec` at alpha `0.85` with
`20px` blur) shows through uniformly. Typed text and program output use
the normal foreground; only the prompt is green (see `Prompt` below).

All text uses the SF family: `SF Mono` at 9pt for the terminal cells
(`SF Mono 9, Adwaita Mono 9, Monospace 9` fallback chain, roughly twice
as many cells as 13pt),
`SF Pro Display` for the decoration bar (UIKit default). System paths
are `/usr/share/fonts/OTF/SF-Pro-Display-Regular.otf` and the Mono
equivalent.

## Sound and bell

| Setting | Value | Effect |
|---|---|---|
| `AUDIBLE_BELL` | `true` | VTE system beep (`set_audible_bell`) |
| `VISUAL_BELL` | `true` | 90ms opacity flash on `connect_bell` |

```rust
pub const AUDIBLE_BELL: bool = true;
pub const VISUAL_BELL: bool = true;
```

The visual flash drops the terminal opacity to `0.75` and restores it
after `90ms` via `glib::timeout_add_local_once`. Both bells fire for
ASCII BEL (`\x07`) and OSC bell sequences handled inside VTE.

## Prompt

Only the prompt is green: `user@machine folder %` in ANSI color 2
(palette `#00c200`, readable on both schemes). Typed text and output
stay in the normal foreground.

`Resources/tontoo-prompt.zsh` defines it in zsh syntax (`%#` renders
`%` for users and `#` for root):

```zsh
PROMPT='%F{2}%n@%m %~ %#%f '
```

The format is enforced by our own `precmd` hook (`_tontoo_prompt`,
registered after theme hooks so it runs last). Themes like grml or
oh-my-zsh rebuild `PROMPT` inside their own hooks and would otherwise
clobber the format; the hook also clears `RPROMPT`/`RPS1`.

zsh reads rc files from `$ZDOTDIR`, so the app points it at a generated
dir (`$XDG_RUNTIME_DIR/tontoo-terminal`, fallback
`/tmp/tontoo-terminal-<uid>`). The generated `.zshenv` / `.zshrc`
source the real files first (`/etc/zsh/*`, then `$TONTOO_REALHOME/.*`)
and the prompt file last, so aliases, completions and themes keep
working while the green prompt wins. Other shells spawn untouched.

The prompt file also reports the current folder to VTE via OSC 7
(`precmd`/`chpwd` hooks, space and percent encoded). Stock zsh never
sends this on its own; without it VTE keeps an empty directory uri and
the title plus title-click fall back to home.

| Function | Behavior |
|---|---|
| `prompt_path()` | Finds `tontoo-prompt.zsh` (bundle `Resources/`, dev checkout, `/usr/share/terminal/`) |
| `prepare_zdotdir(prompt)` | Writes the delegating rc files, returns the dir (`None` when not writable) |

```rust
pub fn prompt_path() -> Option<PathBuf>
pub fn prepare_zdotdir(prompt: &Path) -> Option<PathBuf>
```

`prepare_zdotdir` returns `None` when the runtime dir is not writable;
the shell then starts as plain zsh without the green prompt, never
failing the window.

## Scrollback and input

| Setting | Value |
|---|---|
| `SCROLLBACK_LINES` | `10000` |
| `scroll_on_output` | `true` |
| `scroll_on_keystroke` | `true` |
| `mouse_autohide` | `true` |
| `allow_hyperlink` | `true` |
| `cursor` | Block, system blink |
| `bold_is_bright` | `true` |
| `size` | 80x24 initial |

Keyboard input, clipboard, selection and URL highlighting are native
VTE behavior; there is no separate input row by design.

## Localization

Strings live in `lang/en_us.json` and `lang/de_de.json` (only these
two). `src/lang.rs` detects German from `LANGUAGE`, `LC_ALL`, `LANG`
or `/etc/locale.conf` and falls back to `en_us`.

`Resources/lang/` holds copies of both files: TBuild copies only
`Resources/` into the `.app` bundle (root `lang/` is used for dev runs
and the localized `name` in `Info.tontoo`). Keep both locations in sync.

| Key | en_us | de_de |
|---|---|---|
| `app.title` | `Terminal` | `Terminal` |
| `app.name` | `Terminal` | `Terminal` |
| `terminal.untitled` | `Terminal` | `Terminal` |
| `terminal.shell_exited` | `Shell exited, new shell started` | `Shell beendet, neue Shell gestartet` |
| `title.open_in_finder` | `Open in Finder` | `Im Finder öffnen` |

### `t(key)`

```rust
pub fn t(key: &str) -> String
```

Returns the localized string for `key`. Returns the key itself when the
locale file or key is missing, so the UI never renders empty text.

## Usage / Example

```bash
cargo run
LANG=de_DE.UTF-8 cargo run
TONTOO_TERMINAL_SHELL=/bin/bash cargo run
```

The first command opens `zsh` with English chrome, the second with
German strings, the third forces `bash` instead of `zsh`.

## Packaging

`tontoo.proj` (`bundle_id: com.tontoo.terminal`) lets TBuild assemble
the `.app` bundle:

```bash
tbuild app /path/to/Terminal
```

The bundle contains the release binary (`App/`), the icon
(`Resources/app_icon.png`, 1024x1024) and `Resources/lang/` (`lang/`).
The ISO stages it via `BaseOS/scripts/stage-terminal.sh` (TBuild, like
Weather) into `/Applications/Terminal.app`, with lang plus prompt
fallbacks under `/usr/share/terminal/`.
The runtime lookup covers the bundle layout
(`<Name>.app/Resources/lang`), dev checkouts (`lang/`,
`Resources/`) and installed files (`/usr/share/terminal/`).

## Cross References

- [MAIN.md](MAIN.md) -- wiki entry point
- UIKit `WindowType::Mac` -- taller decoration bar with centered title
- UIKit `set_window_transparency` / `set_window_blur` -- medium glass
- VTE `window-title-changed` -- live program title source
