# Terminal – Wiki

Terminal is the TontooOS terminal emulator: a TontooUI window with a
macOS-style `Titlebar` (traffic lights, live title) on top of a terminal
grid that the app draws itself on Vello. The emulator is built in: a VT
parser, a screen grid with scrollback, the alternate screen, mouse
reporting, bracketed paste, 256 color and truecolor. There is no GTK and
no `vte4` package any more, and no background blur: the window body is
the flat TontooOS background token (`#1b2022` dark, `#ffffff` light). The
bar title follows the running program (OSC 0/1/2), else the current
folder (OSC 7); clicking it opens the folder in Finder. Default shell is
`zsh`.

- Repository: https://github.com/TontooOS/TontooMicroApps
- License: TCL v27.0
- Version: 27.0.0

## Feature Index

| Feature | File | Description |
|---|---|---|
| Main index | [MAIN.md](MAIN.md) | This page |
| Rules | [RULE.md](RULE.md) | Development and usage rules |
| Terminal | [Terminal.md](Terminal.md) | Window, grid, VT parser, input, colors, shell and localization |

## Quick Start

Run the Terminal window from the repository root:

```bash
cargo run
```

The window follows the system theme live through `ThemeWatcher` (Dark
`#1b2022` / Light `#ffffff`, text `#d8d9d9` / `#272727`) and picks
German strings when the locale starts with `de`. `zsh` opens by default;
set `TONTOO_TERMINAL_SHELL` to override, `TONTOO_TERMINAL_FONT_SIZE` to
change the 13 px default.

See [Terminal.md](Terminal.md) for details.

## Changelog

- 2026-10-02: Ported to TontooUI, 1:1 in behavior. Own VT parser and
  screen grid on Vello, no GTK and no `vte4` (the ISO no longer needs
  the `vte4` package), window without background blur. New: alternate
  screen, mouse reporting (1000/1002/1003/1006), bracketed paste,
  256 color and truecolor, DEC Special Graphics, DECALN, scrollback
  selection with copy and paste, drag region window move, live window
  title. Colors follow the TontooOS tokens (`#1b2022` / `#ffffff`,
  text `#d8d9d9` / `#272727`), font is the SF Mono cascade at 13 px.
  TontooUI gained the input hooks this needed: `App::raw_key`,
  `App::mouse_button` and `App::window_title`.
- 2026-09-10: ISO requires `vte4` package (missing lib crashed with exit 127).
- 2026-09-10: Prompt format enforced via precmd hook (wins over grml/oh-my-zsh themes).
- 2026-09-10: OSC 7 cwd reporting in prompt file, title click opens the real folder.
- 2026-09-10: `exit` closes the window, 1170x600 default (1.3x width), ISO staging via TBuild into `/Applications/Terminal.app`.
- 2026-09-10: `user@machine folder %` prompt format, title click opens current folder in Finder.
- 2026-09-10: Green `[user@machine folder]` prompt only (zsh ZDOTDIR delegation, text back to white).
- 2026-09-10: 9pt SF Mono (double content), phosphor green text in Dark mode, home dir via passwd lookup (root lands in `/root`).
- 2026-09-10: Initial Terminal v1 (VTE view, Mac bar with live title, transparent medium glass, zsh default, `lang/en_us.json` and `lang/de_de.json`).