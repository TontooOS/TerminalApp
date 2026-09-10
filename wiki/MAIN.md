# Terminal – Wiki

Terminal is the TontooOS terminal emulator: a 1170x600 UIKit window with
a macOS-style decoration bar (`WindowType::Mac`) and a single VTE
terminal filling the content area. The bar title follows the running
program (OSC 0/1/2 window title), else the current path. The background
is transparent medium glass (alpha `0.85` plus `20px` blur). Default
shell is `zsh`.

- Repository: https://github.com/TontooOS/TontooOS
- License: TCL v26.1
- Version: 26.1.0

## Feature Index

| Feature | File | Description |
|---|---|---|
| Main index | [MAIN.md](MAIN.md) | This page |
| Rules | [RULE.md](RULE.md) | Development and usage rules |
| Terminal | [Terminal.md](Terminal.md) | VTE view, title, colors, bell, shell and localization |

## Quick Start

Run the Terminal window from the repository root:

```bash
cargo run
```

The window follows the GNOME system theme live (Dark `#1d1d1d`, Light
`#ececec`) and picks German strings when `LANG` starts with `de`.
`zsh` opens by default; set `TONTOO_TERMINAL_SHELL` to override.

See [Terminal.md](Terminal.md) for details.

## Changelog

- 2026-09-10: `exit` closes the window, 1170x600 default (1.3x width), ISO staging via TBuild into `/Applications/Terminal.app`.
- 2026-09-10: `user@machine folder %` prompt format, title click opens current folder in Finder.
- 2026-09-10: Green `[user@machine folder]` prompt only (zsh ZDOTDIR delegation, text back to white).
- 2026-09-10: 9pt SF Mono (double content), phosphor green text in Dark mode, home dir via passwd lookup (root lands in `/root`).
- 2026-09-10: Initial Terminal v1 (VTE view, Mac bar with live title, transparent medium glass, zsh default, `lang/en_us.json` and `lang/de_de.json`).
