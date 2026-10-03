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

- 2026-10-03: Default window size is now 804x559, measured from a debug
  run: the window frame takes 24 px per side, so the content viewport is
  756x511 and the grid 96x27 at 13 px.
- 2026-10-03: The right click menu no longer shows a stray "Terminal"
  button on startup. A `Menu` without an anchor draws its button, so it
  is only placed and drawn while it is open; the first right click moved
  it to the pointer and made it disappear.
- 2026-10-03: Right click menu and the real selection paint bug. The
  background fill iterated the style runs and filled only the first
  segment of each one, so every selection that started inside a run was
  never painted; it now walks the row with `background_segments` and a
  selection in a long prompt line shows. Right click opens a glass
  `Copy`, `Paste`, `Select All` menu over the grid instead of pasting,
  the pointer goes to the menu while it is open, and `Select All` stops
  at the last line with content.
- 2026-10-03: Copy and paste on the plain Ctrl chords. A trace run
  (`TONTOO_TERMINAL_DEBUG=1`) showed the shell never reports the shift
  modifier on this system, so `Ctrl+Shift+V` was unreachable and plain
  `Ctrl+V` went to the shell as `0x16`. `Ctrl+V` now always pastes,
  `Ctrl+C` copies when a selection exists and stays the interrupt
  otherwise, and `Ctrl+A` stays "start of line". The selection veil is
  also stronger, it was too subtle to notice against the dark body.
- 2026-10-02: App icon is a Tontoo `.tico` container
  (`Resources/icon.tico`), converted from `Resources/app_icon.png` with
  the CoreIcon `tico_from_png` example: one non-recolorable image layer
  over a transparent background, Apple icon finish applied by
  `TicoIcon::render`. `tontoo.proj` points at the container, the PNG
  stays as the regeneration source.
- 2026-10-02: Selection and clipboard fixes. The background fill now
  splits a style run at the selection boundary, so a selection inside a
  long run (a whole prompt line) is painted instead of silently
  inheriting the unselected state of the run start. Copy and paste moved
  into one `shortcut` resolver and are available on `Super+C`,
  `Super+V`, `Ctrl+Shift+C`, `Ctrl+Shift+V`, `Ctrl+Insert` and
  `Shift+Insert`; `Ctrl+C` and `Ctrl+V` stay `SIGINT` and "quote next".
  New tests cover single character drag selection, backwards drags,
  copy to the clipboard and the shortcut table.

- 2026-10-02: Grid alignment and cursor fix. Grid text is laid out with
  the CoreText monospace generic family instead of a named family list:
  a quoted list such as `SF Mono, monospace` resolves to one missing
  family and fell back to the proportional system font, so the measured
  cell width and the drawn advance disagreed and the cursor sat far
  right of the line. The cursor no longer blinks (static block), only
  SGR 5 and 6 text does.

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