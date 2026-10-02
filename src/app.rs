//! The TontooUI application: window shell, macOS title bar, terminal
//! grid and every input path.
//!
//! The window draws its own chrome the TontooUI way: `Titlebar` for the
//! macOS decoration bar and a plain Vello surface for the grid, both
//! painted by [`TerminalApp::draw`]. No background blur: the grid body
//! is the flat TontooOS background token.

use crate::TontooUI::Color;
use crate::TontooUI::elements::{Titlebar, TitlebarHeight, TrafficAction};
use crate::TontooUI::kurbo::{self, Affine, Rect as KurboRect};
use crate::TontooUI::peniko::{Brush, Fill};
use crate::TontooUI::renderer::window::{
  App as TontooApp, CursorKind, KeyPress, Modifiers, MouseButtonKind, RawKey, Viewport,
  WindowCommand,
};
use crate::TontooUI::renderer::{FontSystem, ImageLoader};
use crate::TontooUI::theme::{ThemeMode, ThemeWatcher};
use crate::TontooUI::Scene;

use crate::config;
use crate::grid::Screen;
use crate::input::{self, KeyInput, PointerAction, Wheel};
use crate::lang;
use crate::parser::Parser;
use crate::prompt;
use crate::pty::{Output, Pty};
use crate::render::{Area, GridRenderer, Selection};
use crate::theme;

/// Grid size before the window reports its viewport.
const BOOT_COLS: u16 = 80;
const BOOT_ROWS: u16 = 24;

/// Extra room around the title text that still counts as the title, so a
/// click there opens the folder in Finder.
const TITLE_CLICK_PAD: f32 = 14.0;

/// Lines a single wheel notch scrolls.
const WHEEL_LINES: i32 = 3;

/// Terminal for TontooOS as a TontooUI app.
pub struct TerminalApp {
  bar: Titlebar,
  renderer: GridRenderer,
  screen: Screen,
  parser: Parser,
  pty: Option<Pty>,
  watcher: ThemeWatcher,
  focused: bool,
  dark: bool,
  area: Area,
  title: String,
  window_title: String,
  command: Option<WindowCommand>,
  selection: Option<Selection>,
  drag_anchor: Option<(usize, u16)>,
  keyboard_anchor: Option<(usize, u16)>,
  press_modifiers: Modifiers,
  pointer: (f64, f64),
  last_click: Option<(f32, f32, f64)>,
  click_count: u32,
  grid: (u16, u16),
  reply: Vec<u8>,
  bell_flash: Option<f64>,
  title_rect: KurboRect,
  title_scale: f32,
  now: f64,
  shell_stopped: bool,
}

impl TerminalApp {
  /// Build the app and start the shell.
  pub fn new() -> Self {
    let mut app = Self {
      bar: Titlebar::new(lang::t("app.title")).height(TitlebarHeight::Mac),
      renderer: GridRenderer::new(),
      screen: Screen::new(BOOT_COLS, BOOT_ROWS),
      parser: Parser::new(),
      pty: None,
      watcher: ThemeWatcher::new(),
      focused: true,
      dark: true,
      area: Area {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
      },
      title: lang::t("app.title"),
      window_title: lang::t("app.title"),
      command: None,
      selection: None,
      drag_anchor: None,
      keyboard_anchor: None,
      press_modifiers: Modifiers::default(),
      pointer: (0.0, 0.0),
      last_click: None,
      click_count: 0,
      grid: (BOOT_COLS, BOOT_ROWS),
      reply: Vec::new(),
      bell_flash: None,
      title_rect: KurboRect::new(0.0, 0.0, 0.0, 0.0),
      title_scale: 0.0,
      now: 0.0,
      shell_stopped: false,
    };
    app.start_shell();
    app
  }

  /// Spawn the configured shell on a PTY.
  fn start_shell(&mut self) {
    let shell = config::resolve_shell();
    let home = config::home_dir();
    let zdotdir = if config::shell_basename(&shell) == "zsh" {
      prompt::prompt_path().and_then(|path| prompt::prepare_zdotdir(&path))
    } else {
      None
    };
    let mut env: Vec<(String, String)> = vec![("TONTOO_REALHOME".to_string(), home.clone())];
    if let Some(dir) = &zdotdir {
      env.push((
        "ZDOTDIR".to_string(),
        dir.display().to_string(),
      ));
    }
    let argv: Vec<String> = vec![shell];
    match Pty::spawn(
      &argv[0],
      &argv[1..].iter().map(String::as_str).collect::<Vec<&str>>(),
      &home,
      self.grid.0,
      self.grid.1,
      &env,
    ) {
      Ok(pty) => self.pty = Some(pty),
      Err(error) => {
        eprintln!("terminal: {error}");
        self.command = Some(WindowCommand::Close);
      }
    }
  }

  /// Drain shell output, feed the parser and write pending replies.
  fn pump_output(&mut self, time_secs: f64) {
    let Some(mut pty) = self.pty.take() else {
      return;
    };
    let mut exited = false;
    while let Some(output) = pty.read_output() {
      match output {
        Output::Data(bytes) => {
          self
            .parser
            .feed(&bytes, &mut self.screen, &mut self.reply, time_secs);
        }
        Output::Exited(status) => {
          let _ = status;
          exited = true;
        }
      }
    }
    if !self.reply.is_empty() {
      let reply = std::mem::take(&mut self.reply);
      if !pty.write(&reply) {
        self.shell_stopped = true;
      }
    }
    if let Some(clipboard) = self.screen.clipboard_write.take() {
      crate::clipboard::set(&clipboard);
    }
    if self.screen.bell_at.take().is_some() && config::VISUAL_BELL {
      self.bell_flash = Some(time_secs);
    }
    self.pty = Some(pty);
    if exited || self.pty.as_ref().map(Pty::exited).unwrap_or(false) {
      self.shell_stopped = true;
      if let Some(status) = self.pty.as_ref().and_then(Pty::status) {
        eprintln!("terminal: shell exited with status {status}");
      }
      // `exit` closes the window, like the old VTE build did.
      self.command = Some(WindowCommand::Close);
    }
  }

  /// Send input to the shell.
  fn write(&mut self, bytes: &[u8]) {
    if self.shell_stopped {
      return;
    }
    if let Some(pty) = self.pty.as_mut() {
      if !pty.write(bytes) {
        self.shell_stopped = true;
      }
    }
    self.screen.scroll_to_bottom();
  }

  /// Keep the grid, the PTY and the window size in sync.
  fn sync_grid(&mut self) {
    let (cols, rows) = self.renderer.grid_size(self.area);
    if (cols, rows) == self.grid || cols == 0 || rows == 0 {
      return;
    }
    self.grid = (cols, rows);
    self.screen.resize(cols, rows);
    if let Some(pty) = self.pty.as_mut() {
      pty.resize(cols, rows);
    }
    self.clamp_selection();
  }

fn clamp_selection(&mut self) {
    let Some(selection) = self.selection else {
      return;
    };
    let last_line = self.screen.history_len() + self.screen.rows as usize - 1;
    if selection.ordered().1 .0 <= last_line {
      return;
    }
    self.selection = None;
  }

  /// Display title: the OSC program title wins, then the reported
  /// folder, then the app name.
  fn sync_title(&mut self) {
    let mut title = self.screen.title.trim().to_string();
    if title.is_empty() {
      let cwd = self.screen.cwd.trim();
      if !cwd.is_empty() {
        title = cwd_path(cwd);
      }
    }
    if title.is_empty() {
      title = lang::t("terminal.untitled");
    }
    if title != self.title {
      self.title = title.clone();
      self.bar.set_title(title.clone());
    }
    if self.title != self.window_title {
      self.window_title = self.title.clone();
    }
  }

  /// Rect of the title text, used for the Finder click. Cached until the
  /// title or the window scale changes. Measured with the same system
  /// font the `Titlebar` draws with, so the rect matches the glyphs.
  fn measure_title_rect(&mut self, fonts: &mut FontSystem) {
    if self.title_scale == fonts.scale && !self.title_rect.is_zero_area() {
      return;
    }
    let (x, y, width, height) = self.bar.bounds();
    let frame = fonts.layout_text_weighted(
      &self.title,
      13.0,
      theme::titlebar_text(self.dark),
      600.0,
      None,
    );
    let (text_width, _) = frame.size();
    let center = x + width / 2.0;
    let half = text_width / 2.0 + TITLE_CLICK_PAD;
    self.title_rect = KurboRect::new(
      (center - half) as f64,
      (y + (height - 20.0) / 2.0) as f64,
      (center + half) as f64,
      (y + (height + 20.0) / 2.0) as f64,
    );
    self.title_scale = fonts.scale;
  }

  /// Grid cell under a pointer position as a virtual line plus column.
  fn cell_under(&self, x: f64, y: f64) -> Option<(usize, u16)> {
    let (column, row) = self.renderer.cell_at(self.area, x as f32, y as f32)?;
    Some((self.screen.view_top() + row as usize, column))
  }

  /// Grid row of a virtual line, for mouse reports.
  fn row_of(&self, line: usize) -> u16 {
    (line - self.screen.view_top()) as u16
  }

  /// Copy the selection to the clipboard.
  fn copy_selection(&mut self) {
    let Some(selection) = self.selection else {
      return;
    };
    let text = selection.text(&self.screen);
    if !text.is_empty() {
      crate::clipboard::set(&text);
    }
  }

  /// Paste the clipboard, wrapped in bracketed paste markers when the
  /// program asked for them.
  fn paste(&mut self) {
    let modes = self.screen.modes;
    let Some(text) = crate::clipboard::get() else {
      return;
    };
    let bytes = input::encode_paste(&text, &modes);
    self.write(&bytes);
  }

  /// Extend the selection to a cell while dragging.
  fn select_to(&mut self, cell: (usize, u16)) {
    let anchor = self.drag_anchor.unwrap_or(cell);
    self.drag_anchor = Some(anchor);
    let selection = Selection::new(anchor, cell);
    self.selection = if selection.ordered().0 == selection.ordered().1 {
      None
    } else {
      Some(selection)
    };
  }

  /// Select the word under the cell (double click).
  fn select_word(&mut self, cell: (usize, u16)) {
    let Some(line) = self.screen.line(cell.0) else {
      return;
    };
    let start = line.word_start(cell.1);
    let end = line.word_end(cell.1);
    self.drag_anchor = Some((cell.0, start));
    self.selection = Some(Selection::new((cell.0, start), (cell.0, end)));
  }

  /// Select the whole line (triple click).
  fn select_line(&mut self, cell: (usize, u16)) {
    let cols = self.screen.cols;
    self.drag_anchor = Some((cell.0, 0));
    self.selection = Some(Selection::new((cell.0, 0), (cell.0, cols)));
  }

  /// Report a mouse event to a program that asked for mouse tracking.
  fn report_mouse(
    &mut self,
    action: PointerAction,
    button: u8,
    x: f64,
    y: f64,
    modifiers: Modifiers,
    wheel: Option<Wheel>,
  ) {
    let Some(cell) = self.cell_under(x, y) else {
      return;
    };
    let modes = self.screen.modes;
    let Some(bytes) = input::encode_mouse(
      &modes,
      action,
      button,
      cell.1 + 1,
      self.row_of(cell.0) + 1,
      modifiers,
      wheel,
    ) else {
      return;
    };
    self.write(&bytes);
  }

  /// Send focus in / out when mode 1004 is active.
  fn report_focus(&mut self, focused: bool) {
    if !self.screen.modes.focus_events {
      return;
    }
    let bytes = input::encode_focus(focused);
    self.write(&bytes);
  }
}

/// Current folder as a plain path from an OSC 7 URI.
pub fn cwd_path(uri: &str) -> String {
  let Some(stripped) = uri.strip_prefix("file://") else {
    return uri.to_string();
  };
  let path = match stripped.find('/') {
    Some(index) => &stripped[index..],
    None => stripped,
  };
  percent_decode(path)
}

fn percent_decode(input: &str) -> String {
  let mut bytes: Vec<u8> = Vec::with_capacity(input.len());
  let raw = input.as_bytes();
  let mut index = 0;
  while index < raw.len() {
    if raw[index] == b'%' && index + 2 < raw.len() {
      if let (Some(high), Some(low)) = (hex_value(raw[index + 1]), hex_value(raw[index + 2])) {
        bytes.push(high * 16 + low);
        index += 3;
        continue;
      }
    }
    bytes.push(raw[index]);
    index += 1;
  }
  String::from_utf8_lossy(&bytes).to_string()
}

fn hex_value(byte: u8) -> Option<u8> {
  match byte {
    b'0'..=b'9' => Some(byte - b'0'),
    b'a'..=b'f' => Some(byte - b'a' + 10),
    b'A'..=b'F' => Some(byte - b'A' + 10),
    _ => None,
  }
}

/// Open a folder in the file manager (native Finder first).
fn open_in_finder(path: &str) {
  if std::process::Command::new("finder").arg(path).spawn().is_ok() {
    return;
  }
  let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

impl TontooApp for TerminalApp {
  fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    _images: &mut ImageLoader<'_>,
    viewport: Viewport,
    time_secs: f64,
  ) {
    self.pump_output(time_secs);
    self.now = time_secs;
    self.watcher.poll(time_secs);
    self.watcher.set_focused(self.focused, time_secs);
    let palette = self.watcher.palette(time_secs);
    let dark = self.watcher.theme().mode == ThemeMode::Dark;
    self.dark = dark;
    self.screen.background_is_light = !dark;

    self.renderer.measure(fonts);
    let bar_height = self.bar.bounds().3;
    self.area = Area {
      x: viewport.x,
      y: viewport.y + bar_height,
      width: viewport.width,
      height: (viewport.height - bar_height).max(0.0),
    };
    self.sync_grid();
    self.sync_title();

    self.bar.set_palette(palette.titlebar_bg, palette.titlebar_text, palette.divider);
    self.bar.set_rect(viewport.x, viewport.y, viewport.width);
    self.bar.set_focused(self.focused);
    self.bar.draw(scene, fonts);
    self.measure_title_rect(fonts);

    self.renderer.draw(
      scene,
      fonts,
      &self.screen,
      self.area,
      dark,
      self.selection,
      self.focused,
      time_secs,
    );

    if let Some(flash) = self.bell_flash {
      if time_secs - flash < config::VISUAL_BELL_SECONDS {
        let scale = fonts.scale as f64;
        let rect = KurboRect::new(
          self.area.x as f64 * scale,
          self.area.y as f64 * scale,
          (self.area.x + self.area.width) as f64 * scale,
          (self.area.y + self.area.height) as f64 * scale,
        );
        scene.fill(
          Fill::NonZero,
          Affine::IDENTITY,
          &Brush::Solid(Color::from_rgba8(255, 255, 255, 42)),
          None,
          &rect,
        );
      } else {
        self.bell_flash = None;
      }
    }
  }

  fn background(&self) -> Color {
    theme::background(self.dark)
  }

  fn window_title(&self) -> Option<&str> {
    Some(&self.window_title)
  }

  fn drag_region(&self) -> Option<(f32, f32, f32, f32)> {
    // The title text stays clickable (it opens the folder in Finder), so
    // the drag region stops in front of it.
    let (x, y, width, height) = self.bar.drag_rect();
    let right = (self.title_rect.x0 as f32 - x).max(0.0);
    if right <= 0.0 {
      return None;
    }
    Some((x, y, right.min(width), height))
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    if let Some(action) = self.bar.press(x as f32, y as f32) {
      self.command = Some(match action {
        TrafficAction::Close => WindowCommand::Close,
        TrafficAction::Minimize => WindowCommand::Minimize,
        TrafficAction::Maximize => WindowCommand::ToggleMaximize,
      });
    }
  }

  fn mouse_move(&mut self, x: f64, y: f64) {
    self.pointer = (x, y);
    self.bar.set_hover(x as f32, y as f32);
    if self.drag_anchor.is_none() {
      return;
    }
    if let Some(cell) = self.cell_under(x, y) {
      self.select_to(cell);
      self.report_mouse(
        PointerAction::Move,
        0,
        x,
        y,
        self.press_modifiers,
        None,
      );
    }
  }

  fn mouse_up(&mut self, _x: f64, _y: f64) {
    self.drag_anchor = None;
  }

  fn mouse_wheel(&mut self, _dx: f64, dy: f64) {
    let notches = (dy / 20.0).round() as i32;
    if notches == 0 {
      return;
    }
    let direction = if dy < 0.0 {
      Wheel::Up
    } else {
      Wheel::Down
    };
    // A program with mouse tracking owns the wheel; without it the
    // scrollback scrolls. Wheel down moves towards the newest output,
    // so the offset shrinks.
    if self.screen.modes.mouse_active() {
      let x = self.pointer.0;
      let y = self.pointer.1;
      for _ in 0..notches.abs() {
        self.report_mouse(
          PointerAction::Move,
          0,
          x,
          y,
          Modifiers::default(),
          Some(direction),
        );
      }
      return;
    }
    if self.screen.scroll_by(-notches * WHEEL_LINES) {
      self.clamp_selection();
    }
  }

  fn cursor(&self, x: f64, y: f64) -> CursorKind {
    match self.renderer.cell_at(self.area, x as f32, y as f32) {
      Some(_) => CursorKind::Text,
      None => CursorKind::Default,
    }
  }

  fn mouse_button(
    &mut self,
    button: MouseButtonKind,
    pressed: bool,
    x: f64,
    y: f64,
    modifiers: Modifiers,
  ) {
    let Some(cell) = self.cell_under(x, y) else {
      // A click on the title opens the current folder in Finder.
      if pressed && self.title_rect.contains(kurbo::Point::new(x, y)) {
        let cwd = if self.screen.cwd.is_empty() {
          config::home_dir()
        } else {
          cwd_path(&self.screen.cwd)
        };
        open_in_finder(&cwd);
        self.last_click = None;
      }
      return;
    };

    let number = match button {
      MouseButtonKind::Left => 0,
      MouseButtonKind::Middle => 1,
      MouseButtonKind::Right => 2,
      MouseButtonKind::Other(_) => 3,
    };

    if !pressed {
      self.drag_anchor = None;
      if button == MouseButtonKind::Left {
        self.report_mouse(PointerAction::Release, number, x, y, modifiers, None);
      }
      return;
    }

    match button {
      MouseButtonKind::Middle | MouseButtonKind::Right => self.paste(),
      MouseButtonKind::Left => {
        self.press_modifiers = modifiers;
        let time = self.now;
        self.click_count = match self.last_click {
          Some((last_x, last_y, last_time))
            if (last_x - x as f32).abs() < 4.0
              && (last_y - y as f32).abs() < 4.0
              && time - last_time < 0.4 =>
          {
            self.click_count + 1
          }
          _ => 1,
        };
        self.last_click = Some((x as f32, y as f32, time));
        if modifiers.shift {
          if self.keyboard_anchor.is_none() && self.selection.is_none() {
            self.drag_anchor = Some(cell);
          }
          if let Some(anchor) = self.drag_anchor {
            self.selection = Some(Selection::new(anchor, cell));
          }
        } else {
          match self.click_count {
            1 => {
              self.drag_anchor = Some(cell);
              self.selection = None;
            }
            2 => self.select_word(cell),
            _ => self.select_line(cell),
          }
        }
        self.report_mouse(PointerAction::Press, number, x, y, modifiers, None);
      }
      MouseButtonKind::Other(_) => {}
    }
  }

  fn set_focused(&mut self, focused: bool) {
    self.focused = focused;
    self.report_focus(focused);
  }

  fn raw_key(&mut self, press: &KeyPress) {
    if !press.pressed {
      return;
    }
    let modes = self.screen.modes;

    if press.modifiers.ctrl && press.modifiers.shift {
      match press.key {
        RawKey::Character(ch) if ch.eq_ignore_ascii_case(&'c') => {
          self.copy_selection();
          return;
        }
        RawKey::Character(ch) if ch.eq_ignore_ascii_case(&'v') => {
          self.paste();
          return;
        }
        RawKey::Character(ch) if ch.eq_ignore_ascii_case(&'a') => {
          let last = self.screen.total_lines().saturating_sub(1);
          self.selection = Some(Selection::new((0, 0), (last, self.screen.cols)));
          return;
        }
        _ => {}
      }
    }

    if press.modifiers.shift && !press.modifiers.ctrl {
      if let Some(head) = shift_selection(&self.screen, press.key) {
        // Extend from the current head so repeated presses grow the
        // selection, not jump back to the cursor.
        let head = match self.selection {
          Some(selection) => {
            let (_, current) = selection.ordered();
            step_selection(&self.screen, current, head)
          }
          None => head,
        };
        let anchor = self
          .keyboard_anchor
          .or(self.selection.map(|selection| selection.ordered().0))
          .unwrap_or((self.screen.cursor_line(), self.screen.cursor_x));
        self.keyboard_anchor = Some(anchor);
        self.selection = Some(Selection::new(anchor, head));
        return;
      }
    }

    let bytes = input::encode_key(
      &KeyInput {
        key: press.key,
        modifiers: press.modifiers,
        text: press.text.clone(),
      },
      &modes,
    );
    if !bytes.is_empty() {
      self.write(&bytes);
    }
  }

  fn poll_window_command(&mut self) -> Option<WindowCommand> {
    self.command.take()
  }
}

/// Move the selection head for Shift+arrows, Home and End. Returns the
/// new head or `None` when the key is not a selection key.
fn shift_selection(screen: &Screen, key: RawKey) -> Option<(usize, u16)> {
  let line = screen.cursor_line();
  let column = screen.cursor_x;
  Some(match key {
    RawKey::Left => (line, column.saturating_sub(1)),
    RawKey::Right => (
      line,
      column.saturating_add(1).min(screen.cols.saturating_sub(1)),
    ),
    RawKey::Up => (line.saturating_sub(1), column),
    RawKey::Down => (line.saturating_add(1), column),
    RawKey::Home => (line, 0),
    RawKey::End => (line, screen.cols.saturating_sub(1)),
    _ => return None,
  })
}

/// Move the selection head by the same delta the key would move the
/// cursor, starting from the current head.
fn step_selection(screen: &Screen, current: (usize, u16), target: (usize, u16)) -> (usize, u16) {
  let delta_line = target.0 as i64 - screen.cursor_line() as i64;
  let delta_column = target.1 as i64 - screen.cursor_x as i64;
  let line = (current.0 as i64 + delta_line).max(0) as usize;
  let column = (current.1 as i64 + delta_column)
    .clamp(0, screen.cols.saturating_sub(1) as i64) as u16;
  (line, column)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::grid::Modes;

  #[test]
  fn cwd_path_strips_the_uri_scheme() {
    assert_eq!(cwd_path("file:///home/user/project"), "/home/user/project");
    assert_eq!(cwd_path("file://host/home"), "/home");
    assert_eq!(cwd_path("file://host"), "host");
    assert_eq!(cwd_path("/plain/path"), "/plain/path");
  }

  #[test]
  fn cwd_path_decodes_percent_escapes() {
    assert_eq!(cwd_path("file:///home/a%20b"), "/home/a b");
    assert_eq!(cwd_path("file:///home/%C3%A4"), "/home/\u{e4}");
  }

  #[test]
  fn shift_selection_moves_the_head() {
    let mut screen = Screen::new(20, 5);
    screen.cursor_x = 4;
    assert_eq!(shift_selection(&screen, RawKey::Left), Some((0, 3)));
    assert_eq!(shift_selection(&screen, RawKey::Right), Some((0, 5)));
    assert_eq!(shift_selection(&screen, RawKey::Home), Some((0, 0)));
    assert_eq!(shift_selection(&screen, RawKey::End), Some((0, 19)));
    assert_eq!(shift_selection(&screen, RawKey::Escape), None);
  }

  #[test]
  fn shift_up_stays_in_the_buffer() {
    let screen = Screen::new(20, 5);
    assert_eq!(shift_selection(&screen, RawKey::Up), Some((0, 0)));
  }

  #[test]
  fn modes_default_to_xterm() {
    let modes = Modes::default();
    assert!(modes.autowrap);
    assert!(modes.cursor_visible);
    assert!(!modes.app_cursor_keys);
    assert!(!modes.mouse_active());
    assert!(!modes.alternate());
  }
}