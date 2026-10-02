//! Terminal grid: cells, attributes, lines, scrollback and the screen
//! operations a VT sequence performs on them.
//!
//! The screen is a fixed `rows` x `cols` matrix of [`Cell`] plus a
//! scrollback ring buffer that only the primary screen feeds. Program
//! output never reallocates per cell: lines are reused, so a long
//! running `top` stays cheap.

use std::collections::VecDeque;

use crate::config;

/// Cell attribute bits.
pub mod flags {
  pub const BOLD: u8 = 1 << 0;
  pub const DIM: u8 = 1 << 1;
  pub const ITALIC: u8 = 1 << 2;
  pub const UNDERLINE: u8 = 1 << 3;
  pub const BLINK: u8 = 1 << 4;
  pub const INVERSE: u8 = 1 << 5;
  pub const HIDDEN: u8 = 1 << 6;
  pub const STRIKE: u8 = 1 << 7;
}

/// Terminal color: default (theme token), palette index or direct RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Color {
  #[default]
  Default,
  Indexed(u8),
  Rgb(u8, u8, u8),
}

/// Character attributes of one cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attrs {
  pub fg: Color,
  pub bg: Color,
  pub flags: u8,
}

impl Default for Attrs {
  fn default() -> Self {
    Self {
      fg: Color::Default,
      bg: Color::Default,
      flags: 0,
    }
  }
}

impl Attrs {
  pub fn is_set(&self, flag: u8) -> bool {
    self.flags & flag != 0
  }

  pub fn set(&mut self, flag: u8, on: bool) {
    if on {
      self.flags |= flag;
    } else {
      self.flags &= !flag;
    }
  }
}

/// One grid cell. `width` is 1 for normal, 2 for a wide (CJK) glyph and
/// 0 for the trailing half of a wide glyph, which holds no character of
/// its own.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
  pub ch: char,
  pub width: u8,
  pub attrs: Attrs,
}

impl Cell {
  pub fn blank(attrs: Attrs) -> Self {
    Self {
      ch: ' ',
      width: 1,
      attrs,
    }
  }
}

/// One terminal row.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
  cells: Vec<Cell>,
}

impl Line {
  pub fn new(cols: u16, attrs: Attrs) -> Self {
    let cell = Cell::blank(attrs);
    Self {
      cells: vec![cell; cols.max(1) as usize],
    }
  }

  pub fn cells(&self) -> &[Cell] {
    &self.cells
  }

  pub fn resize(&mut self, cols: u16, attrs: Attrs) {
    let cols = cols.max(1) as usize;
    if self.cells.len() > cols {
      self.cells.truncate(cols);
      return;
    }
    let cell = Cell::blank(attrs);
    self.cells.resize(cols, cell);
  }

  pub fn clear(&mut self, attrs: Attrs) {
    let cell = Cell::blank(attrs);
    for existing in self.cells.iter_mut() {
      *existing = cell.clone();
    }
  }

  /// Clear `from..to` (column range, `to` exclusive).
  pub fn clear_range(&mut self, from: u16, to: u16, attrs: Attrs) {
    let cell = Cell::blank(attrs);
    let len = self.cells.len();
    let from = (from as usize).min(len);
    let to = (to as usize).min(len);
    for slot in self.cells.iter_mut().take(to).skip(from) {
      *slot = cell.clone();
    }
  }

  pub fn set(&mut self, col: u16, cell: Cell) {
    if let Some(slot) = self.cells.get_mut(col as usize) {
      *slot = cell;
    }
  }

  pub fn cell(&self, col: u16) -> Option<&Cell> {
    self.cells.get(col as usize)
  }

  /// Text of the row with trailing blanks removed.
  pub fn text(&self) -> String {
    let end = self
      .cells
      .iter()
      .rposition(|cell| cell.ch != ' ' && cell.width != 0)
      .map(|index| index + 1)
      .unwrap_or(0);
    let mut out = String::new();
    for cell in &self.cells[..end] {
      if cell.width == 0 {
        continue;
      }
      out.push(if cell.ch == '\0' { ' ' } else { cell.ch });
    }
    out
  }

  /// Text with `left`..`right` columns, blanks inside kept.
  pub fn text_range(&self, left: u16, right: u16) -> String {
    let mut out = String::new();
    let len = self.cells.len();
    let left = (left as usize).min(len);
    let right = (right as usize).min(len).max(left);
    for cell in &self.cells[left..right] {
      if cell.width == 0 {
        continue;
      }
      out.push(if cell.ch == '\0' { ' ' } else { cell.ch });
    }
    out
  }

  /// Next word boundary at or after `col`, used by double click.
  pub fn word_end(&self, col: u16) -> u16 {
    let len = self.cells.len();
    let start = (col as usize).min(len.saturating_sub(1));
    let kind = |cell: &Cell| {
      if cell.ch == ' ' || cell.ch == '\0' {
        0
      } else if cell.ch.is_alphanumeric() || matches!(cell.ch, '_' | '-' | '.' | '/' | '~' | '$' | '+') {
        1
      } else {
        2
      }
    };
    let wanted = kind(&self.cells[start]);
    let mut index = start;
    while index + 1 < len && kind(&self.cells[index + 1]) == wanted && wanted != 0 {
      index += 1;
    }
    (index + 1) as u16
  }

  /// Previous word boundary before `col`, used by double click.
  pub fn word_start(&self, col: u16) -> u16 {
    let len = self.cells.len();
    let start = (col as usize).min(len);
    if start == 0 {
      return 0;
    }
    let kind = |cell: &Cell| {
      if cell.ch == ' ' || cell.ch == '\0' {
        0
      } else if cell.ch.is_alphanumeric() || matches!(cell.ch, '_' | '-' | '.' | '/' | '~' | '$' | '+') {
        1
      } else {
        2
      }
    };
    let mut index = start;
    while index > 0 && kind(&self.cells[index - 1]) == 0 {
      index -= 1;
    }
    if index > 0 {
      let wanted = kind(&self.cells[index - 1]);
      while index > 0 && kind(&self.cells[index - 1]) == wanted && wanted != 0 {
        index -= 1;
      }
    }
    index as u16
  }
}

/// Terminal cursor position and shape (DECSCUSR).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
  Block,
  Underline,
  Bar,
}

impl CursorShape {
  pub fn from_decscusr(param: u64) -> Self {
    match param {
      3 | 4 => Self::Underline,
      5 | 6 => Self::Bar,
      _ => Self::Block,
    }
  }
}

/// Saved cursor state (DECSC / DECRC).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SavedCursor {
  pub x: u16,
  pub y: u16,
  pub attrs: Attrs,
  pub origin: bool,
  pub wrap_pending: bool,
  pub shape: CursorShape,
  pub charset: u8,
}

/// Terminal modes that programs switch with SM / RM and DECSET /
/// DECRST. Defaults match xterm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Modes {
  pub insert: bool,
  pub newline: bool,
  pub origin: bool,
  pub autowrap: bool,
  pub reverse_wrap: bool,
  pub cursor_visible: bool,
  pub cursor_blink: bool,
  pub app_cursor_keys: bool,
  pub app_keypad: bool,
  pub bracketed_paste: bool,
  pub focus_events: bool,
  pub mouse_x10: bool,
  pub mouse_button: bool,
  pub mouse_drag: bool,
  pub mouse_any: bool,
  pub mouse_sgr: bool,
  pub alt_screen: bool,
}

impl Default for Modes {
  fn default() -> Self {
    Self {
      insert: false,
      newline: false,
      origin: false,
      autowrap: true,
      reverse_wrap: false,
      cursor_visible: true,
      cursor_blink: true,
      app_cursor_keys: false,
      app_keypad: false,
      bracketed_paste: false,
      focus_events: false,
      mouse_x10: false,
      mouse_button: false,
      mouse_drag: false,
      mouse_any: false,
      mouse_sgr: false,
      alt_screen: false,
    }
  }
}

impl Modes {
  /// Any mode that wants pointer reports at all.
  pub fn mouse_active(&self) -> bool {
    self.mouse_x10 || self.mouse_button || self.mouse_drag || self.mouse_any
  }

  /// True while the alternate screen is active (full screen programs).
  pub fn alternate(&self) -> bool {
    self.alt_screen
  }
}

/// The whole terminal state: grid, scrollback, cursor, modes, title and
/// working directory.
pub struct Screen {
  pub cols: u16,
  pub rows: u16,
  pub cursor_x: u16,
  pub cursor_y: u16,
  pub shape: CursorShape,
  pub attrs: Attrs,
  pub modes: Modes,
  pub title: String,
  pub icon_title: String,
  pub cwd: String,
  /// Hyperlink target of the next printed run (OSC 8).
  pub hyperlink: Option<String>,
  /// Clipboard payload of an OSC 52 write, handled by the app.
  pub clipboard_write: Option<String>,
  pub wrap_pending: bool,
  pub scroll_offset: usize,
  pub bell_at: Option<f64>,
  /// True when the system theme is light: OSC 10 / 11 queries must
  /// answer with the light token.
  pub background_is_light: bool,
  scrollback: VecDeque<Line>,
  primary_lines: Vec<Line>,
  alt_lines: Option<Vec<Line>>,
  alt_cursor: Option<(u16, u16)>,
  saved: Option<SavedCursor>,
  scroll_top: u16,
  scroll_bottom: u16,
  tabs: Vec<bool>,
  /// G0 charset (selected with `ESC ( x`).
  g0: u8,
  /// G1 charset (selected with `ESC ) x`).
  g1: u8,
  /// True between SI and SO: G1 is active.
  shift: bool,
}

impl Screen {
  pub fn new(cols: u16, rows: u16) -> Self {
    let cols = cols.max(2);
    let rows = rows.max(1);
    let blank = Attrs::default();
    Self {
      cols,
      rows,
      cursor_x: 0,
      cursor_y: 0,
      shape: CursorShape::Block,
      attrs: Attrs::default(),
      modes: Modes::default(),
      title: String::new(),
      icon_title: String::new(),
      cwd: String::new(),
      hyperlink: None,
      clipboard_write: None,
      wrap_pending: false,
      scroll_offset: 0,
      bell_at: None,
      background_is_light: false,
      scrollback: VecDeque::new(),
      primary_lines: vec![Line::new(cols, blank); rows as usize],
      alt_lines: None,
      alt_cursor: None,
      saved: None,
      scroll_top: 0,
      scroll_bottom: rows - 1,
      tabs: Vec::new(),
      g0: 0,
      g1: 0,
      shift: false,
    }
    .with_default_tabs()
  }

  fn with_default_tabs(mut self) -> Self {
    self.tabs = vec![false; self.cols as usize];
    let mut col = 8;
    while col < self.cols {
      self.tabs[col as usize] = true;
      col += 8;
    }
    self
  }

  pub fn history_len(&self) -> usize {
    self.scrollback.len()
  }

  /// Total number of addressable lines: scrollback plus the visible
  /// grid. Line `0` is the oldest scrollback line.
  pub fn total_lines(&self) -> usize {
    self.scrollback.len() + self.rows as usize
  }

  fn lines(&self) -> &Vec<Line> {
    match self.alt_lines {
      Some(_) => self.alt_lines.as_ref().expect("alt lines"),
      None => &self.primary_lines,
    }
  }

  fn lines_mut(&mut self) -> &mut Vec<Line> {
    match self.alt_lines.is_some() {
      true => self.alt_lines.as_mut().expect("alt lines"),
      false => &mut self.primary_lines,
    }
  }

  /// Line `index` of the virtual buffer (0 = oldest scrollback line).
  pub fn line(&self, index: usize) -> Option<&Line> {
    let history = self.scrollback.len();
    if index < history {
      self.scrollback.get(index)
    } else {
      self.lines().get(index - history)
    }
  }

  /// Cursor line in the virtual buffer.
  pub fn cursor_line(&self) -> usize {
    self.scrollback.len() + self.cursor_y as usize
  }

  /// Scroll the view so the cursor is visible again.
  pub fn scroll_to_bottom(&mut self) {
    self.scroll_offset = 0;
  }

  /// Wheel scrolling: positive `lines` moves the view up (back in
  /// history). Returns true when the view moved.
  pub fn scroll_by(&mut self, lines: i32) -> bool {
    let history = self.scrollback.len() as i64;
    let current = self.scroll_offset as i64;
    let target = (current + lines as i64).clamp(0, history);
    if target == current {
      return false;
    }
    self.scroll_offset = target as usize;
    true
  }

  /// Line index of the first visible row.
  pub fn view_top(&self) -> usize {
    let total = self.total_lines();
    let rows = self.rows as usize;
    total.saturating_sub(rows + self.scroll_offset)
  }

  /// Keep the view pinned to the same content while output scrolls the
  /// screen: new scrollback lines move the window with them.
  pub fn pin_view(&mut self) {
    if self.scroll_offset > 0 && !self.modes.alternate() {
      self.scroll_offset = (self.scroll_offset + 1).min(self.scrollback.len());
    }
  }

  fn push_scrollback(&mut self, line: Line) {
    if self.modes.alternate() {
      return;
    }
    self.scrollback.push_back(line);
    let limit = config::SCROLLBACK_LINES;
    while self.scrollback.len() > limit {
      self.scrollback.pop_front();
    }
  }

  /// Resize the grid, preserving content where possible. Growing or
  /// shrinking pulls lines through the scrollback so a terminal window
  /// resize does not lose output.
  pub fn resize(&mut self, cols: u16, rows: u16) {
    let cols = cols.max(2);
    let rows = rows.max(1);
    if cols == self.cols && rows == self.rows {
      return;
    }
    let blank = Attrs::default();
    if rows < self.rows {
      let drop = (self.rows - rows) as usize;
      let mut removed = 0;
      while removed < drop {
        let history = self.scrollback.len();
        if history == 0 {
          break;
        }
        self.scrollback.pop_front();
        if self.scroll_offset > 0 {
          self.scroll_offset -= 1;
        }
        removed += 1;
      }
    } else if rows > self.rows {
      let add = (rows - self.rows) as usize;
      // Pull what the scrollback can give, then blank the rest at the
      // bottom, so the visible content stays where it was.
      let history = self.scrollback.len();
      let mut added = 0;
      while added < add {
        if history == 0 {
          break;
        }
        if let Some(line) = self.scrollback.pop_back() {
          self.lines_mut().insert(0, line);
          added += 1;
        }
      }
      if self.alt_lines.is_none() {
        while self.primary_lines.len() < rows as usize {
          self.primary_lines.push(Line::new(cols, blank));
        }
      } else if let Some(alt) = self.alt_lines.as_mut() {
        while alt.len() < rows as usize {
          alt.push(Line::new(cols, blank));
        }
      }
    }
    if cols != self.cols {
      for line in self.primary_lines.iter_mut() {
        line.resize(cols, blank);
      }
      if let Some(alt) = self.alt_lines.as_mut() {
        for line in alt.iter_mut() {
          line.resize(cols, blank);
        }
      }
      self.cols = cols;
      self.tabs = vec![false; cols as usize];
      let mut col = 8;
      while col < cols {
        self.tabs[col as usize] = true;
        col += 8;
      }
      for line in self.scrollback.iter_mut() {
        line.resize(cols, blank);
      }
    }
    self.rows = rows;
    self.cursor_x = self.cursor_x.min(cols - 1);
    self.cursor_y = self.cursor_y.min(rows - 1);
    self.scroll_top = 0;
    self.scroll_bottom = rows - 1;
    self.scroll_offset = 0;
  }

  /// Full reset (RIS): fresh grid, defaults, no scrollback.
  pub fn reset(&mut self) {
    let cols = self.cols;
    let rows = self.rows;
    let alt = self.modes.alternate();
    *self = Screen::new(cols, rows);
    if alt {
      self.modes.alt_screen = true;
      self.alt_lines = Some(vec![Line::new(cols, Attrs::default()); rows as usize]);
    }
  }

  /// Switch to or from the alternate screen (`ESC [ ? 1049 h/l`).
  pub fn set_alt_screen(&mut self, active: bool, clear: bool) {
    if active == self.modes.alternate() {
      if active && clear {
        self.clear_screen_keep_cursor();
      }
      return;
    }
    if active {
      self.alt_lines = Some(vec![Line::new(self.cols, Attrs::default()); self.rows as usize]);
      self.alt_cursor = Some((self.cursor_x, self.cursor_y));
      self.modes.alt_screen = true;
      self.cursor_x = 0;
      self.cursor_y = 0;
    } else {
      self.alt_lines = None;
      if let Some((x, y)) = self.alt_cursor.take() {
        self.cursor_x = x;
        self.cursor_y = y;
      }
      self.modes.alt_screen = false;
    }
    self.scroll_offset = 0;
  }

  /// Printable character with wrapping, insert mode and wide glyphs.
  /// The active charset (G0 or G1) is translated here, so the parser
  /// only has to decide which one is active.
  pub fn print(&mut self, ch: char) {
    let charset = if self.shift { self.g1 } else { self.g0 };
    let ch = translate(ch, charset);
    let width = char_width(ch);
    if width == 0 {
      return;
    }
    if self.wrap_pending && self.modes.autowrap {
      self.cursor_x = 0;
      self.index();
      self.wrap_pending = false;
    }
    if width == 2 && self.cursor_x + 1 >= self.cols {
      if self.modes.autowrap {
        self.cursor_x = 0;
        self.index();
      } else {
        return;
      }
    }
    if self.cursor_x >= self.cols {
      self.cursor_x = self.cols - 1;
    }
    if self.modes.insert {
      self.insert_blank(self.cursor_y, self.cursor_x, 1);
    }
    let attrs = self.attrs;
    let line_index = self.cursor_y as usize;
    let x = self.cursor_x;
    if width == 2 {
      let next = x + 1;
      if let Some(line) = self.lines_mut().get_mut(line_index) {
        line.set(x, Cell {
          ch,
          width: 2,
          attrs,
        });
        line.set(next, Cell {
          ch: '\0',
          width: 0,
          attrs,
        });
      }
    } else if let Some(line) = self.lines_mut().get_mut(line_index) {
      line.set(x, Cell {
        ch,
        width: 1,
        attrs,
      });
    }
    if x + width as u16 >= self.cols {
      self.wrap_pending = true;
    } else {
      self.cursor_x = x + width as u16;
    }
  }

  /// Carriage return plus optional line feed.
  pub fn line_feed(&mut self) {
    if self.modes.newline {
      self.cursor_x = 0;
    }
    self.index();
  }

  /// Carriage return.
  pub fn carriage_return(&mut self) {
    self.cursor_x = 0;
    self.wrap_pending = false;
  }

  /// Cursor up one row, stopping at the top margin.
  pub fn cursor_up(&mut self, count: u16) {
    self.cursor_y = self.cursor_y.saturating_sub(count.max(1));
    self.wrap_pending = false;
  }

  /// Cursor down one row, stopping at the bottom margin.
  pub fn cursor_down(&mut self, count: u16) {
    let bottom = self.scroll_bottom;
    self.cursor_y = self.cursor_y.saturating_add(count.max(1)).min(bottom);
    self.wrap_pending = false;
  }

  pub fn cursor_forward(&mut self, count: u16) {
    self.cursor_x = self.cursor_x.saturating_add(count.max(1)).min(self.cols - 1);
    self.wrap_pending = false;
  }

  pub fn cursor_back(&mut self, count: u16) {
    self.cursor_x = self.cursor_x.saturating_sub(count.max(1));
    self.wrap_pending = false;
  }

  /// Move to `row`, `col`, honouring origin mode (DECOM).
  pub fn goto(&mut self, row: u16, col: u16) {
    let (top, bottom) = if self.modes.origin {
      (self.scroll_top, self.scroll_bottom)
    } else {
      (0, self.rows - 1)
    };
    let y = top.saturating_add(row).min(bottom);
    self.cursor_y = y;
    self.cursor_x = col.min(self.cols - 1);
    self.wrap_pending = false;
  }

  /// Absolute row (VPA), no origin offset.
  pub fn move_to_row(&mut self, row: u16) {
    self.cursor_y = row.min(self.rows - 1);
    self.wrap_pending = false;
  }

  /// Absolute column (CHA / HPA), no origin offset.
  pub fn move_to_column(&mut self, col: u16) {
    self.cursor_x = col.min(self.cols - 1);
    self.wrap_pending = false;
  }

  /// Fill the whole grid with `ch` (DECALN, `CSI # 8`).
  pub fn fill_rectangle(&mut self, ch: char) {
    let attrs = self.attrs;
    let cols = self.cols;
    let cell = Cell {
      ch,
      width: 1,
      attrs,
    };
    let line = Line {
      cells: vec![cell; cols as usize],
    };
    let count = self.rows as usize;
    match self.alt_lines.as_mut() {
      Some(alt) => *alt = vec![line; count],
      None => self.primary_lines = vec![line; count],
    }
    self.cursor_x = 0;
    self.cursor_y = 0;
    self.wrap_pending = false;
  }

  /// Move down one row, scrolling the region when at the bottom.
  pub fn index(&mut self) {
    if self.cursor_y == self.scroll_bottom {
      self.scroll_up(1);
    } else if self.cursor_y < self.rows - 1 {
      self.cursor_y += 1;
    }
    self.wrap_pending = false;
  }

  /// Move up one row, scrolling the region when at the top.
  pub fn reverse_index(&mut self) {
    if self.cursor_y == self.scroll_top {
      self.scroll_down(1);
    } else if self.cursor_y > 0 {
      self.cursor_y -= 1;
    }
    self.wrap_pending = false;
  }

  /// Scroll the region up: the top line moves into the scrollback
  /// (primary screen only).
  pub fn scroll_up(&mut self, count: u16) {
    let count = count.max(1) as usize;
    let top = self.scroll_top as usize;
    let bottom = self.scroll_bottom as usize;
    let height = bottom.saturating_sub(top) + 1;
    let count = count.min(height);
    if count == 0 {
      return;
    }
    let cols = self.cols;
    let mut removed: Vec<Line> = Vec::new();
    {
      let lines = self.lines_mut();
      for _ in 0..count {
        if top < lines.len() {
          removed.push(lines.remove(top));
        }
      }
      while lines.len() > bottom + 1 {
        lines.remove(bottom);
      }
      for _ in 0..removed.len() {
        lines.insert(top, Line::new(cols, Attrs::default()));
      }
    }
    for line in removed {
      self.push_scrollback(line);
    }
    self.pin_view();
  }

  /// Scroll the region down: blank lines enter at the top.
  pub fn scroll_down(&mut self, count: u16) {
    let count = count.max(1) as usize;
    let top = self.scroll_top as usize;
    let bottom = self.scroll_bottom as usize;
    let height = bottom.saturating_sub(top) + 1;
    let count = count.min(height);
    let blank = Line::new(self.cols, Attrs::default());
    let lines = self.lines_mut();
    for _ in 0..count {
      if bottom < lines.len() {
        lines.remove(bottom);
      }
      lines.insert(top, blank.clone());
    }
  }

  /// Erase in display: 0 to end, 1 to start, 2 all, 3 scrollback.
  pub fn erase_in_display(&mut self, mode: u64) {
    let cols = self.cols;
    let attrs = self.attrs;
    let row = self.cursor_y;
    let col = self.cursor_x;
    match mode {
      0 => {
        if let Some(line) = self.lines_mut().get_mut(row as usize) {
          line.clear_range(col, cols, attrs);
        }
        for index in (row as usize + 1)..self.lines().len() {
          if let Some(line) = self.lines_mut().get_mut(index) {
            line.clear(attrs);
          }
        }
      }
      1 => {
        if let Some(line) = self.lines_mut().get_mut(row as usize) {
          line.clear_range(0, col + 1, attrs);
        }
        for index in 0..row as usize {
          if let Some(line) = self.lines_mut().get_mut(index) {
            line.clear(attrs);
          }
        }
      }
      3 => {
        self.scrollback.clear();
        self.scroll_offset = 0;
      }
      _ => {
        let blank = Line::new(cols, Attrs::default());
        let count = self.rows as usize;
        match self.alt_lines.as_mut() {
          Some(alt) => *alt = vec![blank; count],
          None => self.primary_lines = vec![blank; count],
        }
      }
    }
  }

  /// Clear the whole screen without moving the cursor.
  pub fn clear_screen_keep_cursor(&mut self) {
    let (x, y) = (self.cursor_x, self.cursor_y);
    self.erase_in_display(2);
    self.cursor_x = x;
    self.cursor_y = y;
  }

  /// Erase in line: 0 to end, 1 to start, 2 all.
  pub fn erase_in_line(&mut self, mode: u64) {
    let attrs = self.attrs;
    let row = self.cursor_y as usize;
    let col = self.cursor_x;
    let cols = self.cols;
    let Some(line) = self.lines_mut().get_mut(row) else {
      return;
    };
    match mode {
      0 => line.clear_range(col, cols, attrs),
      1 => line.clear_range(0, col + 1, attrs),
      _ => line.clear(attrs),
    }
  }

  /// Insert `count` blanks at the cursor column.
  pub fn insert_blank(&mut self, row: u16, col: u16, count: u16) {
    let cols = self.cols;
    let attrs = self.attrs;
    let Some(line) = self.lines_mut().get_mut(row as usize) else {
      return;
    };
    let count = (count as usize).min(cols.saturating_sub(col) as usize);
    line.cells.reserve(count);
    for _ in 0..count {
      line.cells.insert(col as usize, Cell::blank(attrs));
    }
    line.cells.truncate(cols as usize);
  }

  /// Delete `count` cells at the cursor column, pulling the rest left.
  pub fn delete_chars(&mut self, row: u16, col: u16, count: u16) {
    let cols = self.cols;
    let attrs = self.attrs;
    let Some(line) = self.lines_mut().get_mut(row as usize) else {
      return;
    };
    let start = (col as usize).min(line.cells.len());
    let count = (count as usize).min(line.cells.len().saturating_sub(start));
    line.cells.drain(start..start + count);
    let blank = Cell::blank(attrs);
    while line.cells.len() < cols as usize {
      line.cells.push(blank.clone());
    }
    line.cells.truncate(cols as usize);
  }

  /// Erase `count` cells at the cursor column (ECH).
  pub fn erase_chars(&mut self, count: u16) {
    let attrs = self.attrs;
    let row = self.cursor_y;
    let col = self.cursor_x;
    let end = col.saturating_add(count.max(1));
    if let Some(line) = self.lines_mut().get_mut(row as usize) {
      line.clear_range(col, end, attrs);
    }
  }

  /// Insert `count` lines at the cursor row inside the region.
  pub fn insert_lines(&mut self, count: u16) {
    if self.cursor_y < self.scroll_top || self.cursor_y > self.scroll_bottom {
      return;
    }
    let top = self.cursor_y as usize;
    let bottom = self.scroll_bottom as usize;
    let blank = Line::new(self.cols, Attrs::default());
    let lines = self.lines_mut();
    let count = (count.max(1) as usize).min(bottom.saturating_sub(top) + 1);
    for _ in 0..count {
      lines.insert(top, blank.clone());
    }
    while lines.len() > bottom + 1 {
      lines.remove(bottom);
    }
  }

  /// Delete `count` lines at the cursor row inside the region.
  pub fn delete_lines(&mut self, count: u16) {
    if self.cursor_y < self.scroll_top || self.cursor_y > self.scroll_bottom {
      return;
    }
    let top = self.cursor_y as usize;
    let bottom = self.scroll_bottom as usize;
    let cols = self.cols;
    let count = (count.max(1) as usize).min(bottom.saturating_sub(top) + 1);
    let lines = self.lines_mut();
    for _ in 0..count {
      lines.remove(top);
    }
    while lines.len() <= bottom {
      lines.push(Line::new(cols, Attrs::default()));
    }
  }

  /// Set the horizontal scrolling region (DECSTBM).
  pub fn set_scroll_region(&mut self, top: u16, bottom: u16) {
    let top = top.min(self.rows - 1);
    let bottom = bottom.min(self.rows - 1);
    if bottom <= top || bottom == 0 {
      self.scroll_top = 0;
      self.scroll_bottom = self.rows - 1;
    } else {
      self.scroll_top = top;
      self.scroll_bottom = bottom;
    }
    self.goto_home();
  }

  /// Cursor to the home position (1-based CUP coordinates).
  pub fn goto_home(&mut self) {
    self.goto(0, 0);
  }

  /// Tab to the next tab stop.
  pub fn tab_forward(&mut self, count: u16) {
    let mut x = self.cursor_x;
    for _ in 0..count.max(1) {
      let mut next = x.saturating_add(1);
      while next < self.cols && !self.tab_at(next) {
        next += 1;
      }
      x = next.min(self.cols - 1);
    }
    self.cursor_x = x;
    self.wrap_pending = false;
  }

  /// Tab to the previous tab stop.
  pub fn tab_backward(&mut self, count: u16) {
    let mut x = self.cursor_x;
    for _ in 0..count.max(1) {
      let mut next = x.saturating_sub(1);
      while next > 0 && !self.tab_at(next) {
        next -= 1;
      }
      x = next;
    }
    self.cursor_x = x;
    self.wrap_pending = false;
  }

  fn tab_at(&self, col: u16) -> bool {
    self.tabs.get(col as usize).copied().unwrap_or(false)
  }

  /// Set a tab stop at the cursor column (HTS).
  pub fn set_tab(&mut self) {
    if let Some(slot) = self.tabs.get_mut(self.cursor_x as usize) {
      *slot = true;
    }
  }

  /// Clear tab stops: 0 at the cursor, 3 all.
  pub fn clear_tabs(&mut self, mode: u64) {
    match mode {
      3 => self.tabs.iter_mut().for_each(|slot| *slot = false),
      _ => {
        if let Some(slot) = self.tabs.get_mut(self.cursor_x as usize) {
          *slot = false;
        }
      }
    }
  }

  /// Save the cursor (DECSC).
  pub fn save_cursor(&mut self) {
    self.saved = Some(SavedCursor {
      x: self.cursor_x,
      y: self.cursor_y,
      attrs: self.attrs,
      origin: self.modes.origin,
      wrap_pending: self.wrap_pending,
      shape: self.shape,
      charset: self.active_charset(),
    });
  }

  /// Restore the cursor (DECRC).
  pub fn restore_cursor(&mut self) {
    if let Some(saved) = self.saved {
      self.cursor_x = saved.x.min(self.cols - 1);
      self.cursor_y = saved.y.min(self.rows - 1);
      self.attrs = saved.attrs;
      self.modes.origin = saved.origin;
      self.wrap_pending = saved.wrap_pending;
      self.shape = saved.shape;
      self.g0 = saved.charset;
      self.shift = false;
    } else {
      self.goto_home();
      self.attrs = Attrs::default();
    }
  }

  /// Designate a charset: slot 0 is G0 (`ESC ( x`), slot 1 is G1
  /// (`ESC ) x`).
  pub fn designate(&mut self, slot: u8, charset: u8) {
    match slot {
      0 => self.g0 = charset,
      _ => self.g1 = charset,
    }
  }

  /// SI / SO: switch between G1 and G0.
  pub fn invoke(&mut self, shifted: bool) {
    self.shift = shifted;
  }

  /// Currently active charset: 0 is ASCII, 1 is DEC Special Graphics.
  pub fn active_charset(&self) -> u8 {
    if self.shift {
      self.g1
    } else {
      self.g0
    }
  }

  pub fn bell(&mut self, now_secs: f64) {
    self.bell_at = Some(now_secs);
  }

  /// Cursor position report for DSR 6.
  pub fn cursor_position_report(&self) -> String {
    format!("{};{}", self.cursor_y + 1, self.cursor_x + 1)
  }
}

/// Display width of a character in cells: 0 for combining marks, 2 for
/// East Asian wide glyphs, 1 otherwise.
pub fn char_width(ch: char) -> usize {
  let code = ch as u32;
  if code == 0 {
    return 0;
  }
  if code < 0x20 || (0x7f..0xa0).contains(&code) {
    return 0;
  }
  if is_combining(code) {
    return 0;
  }
  if is_wide(code) {
    return 2;
  }
  1
}

fn is_combining(code: u32) -> bool {
  matches!(code,
    0x0300..=0x036f
      | 0x0483..=0x0489
      | 0x0591..=0x05bd
      | 0x0610..=0x061a
      | 0x064b..=0x065f
      | 0x0670
      | 0x06d6..=0x06dc
      | 0x0730..=0x074a
      | 0x07a6..=0x07b0
      | 0x0816..=0x0819
      | 0x08e3..=0x0903
      | 0x093a..=0x093c
      | 0x0941..=0x0948
      | 0x0951..=0x0957
      | 0x0e31
      | 0x0e34..=0x0e3a
      | 0x0e47..=0x0e4e
      | 0x1ab0..=0x1aff
      | 0x1dc0..=0x1dff
      | 0x200b..=0x200f
      | 0x2028..=0x202e
      | 0x2060..=0x2064
      | 0x20d0..=0x20f0
      | 0xfe00..=0xfe0f
      | 0xfe20..=0xfe2f
      | 0xfeff
      | 0xe0100..=0xe01ef)
}

fn is_wide(code: u32) -> bool {
  matches!(code,
    0x1100..=0x115f
      | 0x2329..=0x232a
      | 0x2e80..=0x303e
      | 0x3041..=0x33ff
      | 0x3400..=0x4dbf
      | 0x4e00..=0x9fff
      | 0xa000..=0xa4cf
      | 0xa960..=0xa97f
      | 0xac00..=0xd7a3
      | 0xf900..=0xfaff
      | 0xfe10..=0xfe19
      | 0xfe30..=0xfe6f
      | 0xff00..=0xff60
      | 0xffe0..=0xffe6
      | 0x1f004
      | 0x1f0cf
      | 0x1f18e..=0x1f18f
      | 0x1f191..=0x1f19a
      | 0x1f200..=0x1f320
      | 0x1f32d..=0x1f335
      | 0x1f337..=0x1f37c
      | 0x1f37e..=0x1f393
      | 0x1f3a0..=0x1f3ca
      | 0x1f3cf..=0x1f3d3
      | 0x1f3e0..=0x1f3f0
      | 0x1f3f4
      | 0x1f3f8..=0x1f43e
      | 0x1f440
      | 0x1f442..=0x1f4fc
      | 0x1f4ff..=0x1f53d
      | 0x1f54b..=0x1f54e
      | 0x1f550..=0x1f567
      | 0x1f57a
      | 0x1f595..=0x1f596
      | 0x1f5a4
      | 0x1f5fb..=0x1f64f
      | 0x1f680..=0x1f6c5
      | 0x1f6cc
      | 0x1f6d0..=0x1f6d2
      | 0x1f6eb..=0x1f6ec
      | 0x1f6f4..=0x1f6fc
      | 0x1f7e0..=0x1f7eb
      | 0x1f90c..=0x1f93a
      | 0x1f93c..=0x1f945
      | 0x1f947..=0x1f9ff
      | 0x1fa70..=0x1faff
      | 0x20000..=0x3fffd)
}

/// DEC Special Graphics: the line drawing set of the 0 charset, mapped
/// to the Unicode box drawing block.
fn special_graphics(ch: char) -> Option<char> {
  Some(match ch {
    '_' => '\u{00a0}',
    '`' => '\u{25c6}',
    'a' => '\u{2592}',
    'b' => '\u{2409}',
    'c' => '\u{240c}',
    'd' => '\u{240d}',
    'e' => '\u{240a}',
    'f' => '\u{00b0}',
    'g' => '\u{00b1}',
    'h' => '\u{2424}',
    'i' => '\u{240b}',
    'j' => '\u{2518}',
    'k' => '\u{2510}',
    'l' => '\u{250c}',
    'm' => '\u{2514}',
    'n' => '\u{253c}',
    'o' => '\u{23ba}',
    'p' => '\u{23bb}',
    'q' => '\u{2500}',
    'r' => '\u{23bc}',
    's' => '\u{23bd}',
    't' => '\u{251c}',
    'u' => '\u{2524}',
    'v' => '\u{2534}',
    'w' => '\u{252c}',
    'x' => '\u{2502}',
    'y' => '\u{2264}',
    'z' => '\u{2265}',
    '{' => '\u{03c0}',
    '|' => '\u{2260}',
    '}' => '\u{00a3}',
    '~' => '\u{00b7}',
    _ => return None,
  })
}

/// Translate a printable character through the active charset.
pub fn translate(ch: char, charset: u8) -> char {
  match charset {
    1 => special_graphics(ch).unwrap_or(ch),
    _ => ch,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn screen() -> Screen {
    Screen::new(20, 5)
  }

  fn typed(screen: &mut Screen, text: &str) {
    for ch in text.chars() {
      screen.print(ch);
    }
  }

  #[test]
  fn printing_moves_the_cursor_and_wraps() {
    let mut screen = screen();
    typed(&mut screen, "abc");
    assert_eq!(screen.cursor_x, 3);
    assert_eq!(screen.line(0).unwrap().text(), "abc");
    typed(&mut screen, &"x".repeat(18));
    assert_eq!(screen.cursor_y, 1, "wrap moves to the next row");
    assert_eq!(screen.line(0).unwrap().text().len(), 20);
  }

  #[test]
  fn wide_glyphs_take_two_cells() {
    let mut screen = screen();
    typed(&mut screen, "a日b");
    assert_eq!(screen.cursor_x, 4);
    let line = screen.line(0).unwrap();
    assert_eq!(line.cell(1).unwrap().width, 2);
    assert_eq!(line.cell(2).unwrap().width, 0);
  }

  #[test]
  fn scroll_moves_lines_into_the_scrollback() {
    let mut screen = screen();
    for row in 0..8 {
      for ch in format!("line{row}").chars() {
        screen.print(ch);
      }
      screen.carriage_return();
      screen.line_feed();
    }
    assert_eq!(screen.history_len(), 4, "eight lines on a five row screen");
    assert_eq!(screen.line(0).unwrap().text(), "line0");
    assert_eq!(screen.line(screen.cursor_line()).unwrap().text(), "line7");
  }

  #[test]
  fn alt_screen_hides_the_primary_grid() {
    let mut screen = screen();
    typed(&mut screen, "primary");
    screen.set_alt_screen(true, true);
    typed(&mut screen, "alt");
    assert_eq!(screen.line(screen.cursor_line()).unwrap().text(), "alt");
    screen.set_alt_screen(false, false);
    assert_eq!(screen.line(screen.cursor_line()).unwrap().text(), "primary");
    assert_eq!(screen.cursor_x, 7);
  }

  #[test]
  fn scroll_region_limits_index() {
    let mut screen = screen();
    screen.set_scroll_region(1, 3);
    screen.cursor_y = 3;
    screen.index();
    assert_eq!(screen.cursor_y, 3, "index at the bottom scrolls");
    assert_eq!(screen.history_len(), 1);
    screen.cursor_y = 1;
    screen.reverse_index();
    assert_eq!(screen.cursor_y, 1);
  }

  #[test]
  fn insert_and_delete_shift_cells() {
    let mut screen = screen();
    typed(&mut screen, "abcdef");
    screen.cursor_x = 2;
    screen.insert_blank(screen.cursor_y, 2, 2);
    assert_eq!(screen.line(0).unwrap().text(), "ab  cdef");
    screen.delete_chars(screen.cursor_y, 2, 2);
    assert_eq!(screen.line(0).unwrap().text(), "abcdef");
  }

  #[test]
  fn erase_display_clears_below_the_cursor() {
    let mut screen = screen();
    typed(&mut screen, "hello");
    screen.carriage_return();
    screen.line_feed();
    typed(&mut screen, "world");
    screen.erase_in_display(2);
    assert_eq!(screen.line(0).unwrap().text(), "");
    assert_eq!(screen.line(1).unwrap().text(), "");
  }

  #[test]
  fn tabs_walk_between_stops() {
    let mut screen = screen();
    screen.cursor_x = 0;
    screen.tab_forward(1);
    assert_eq!(screen.cursor_x, 8);
    screen.tab_forward(1);
    assert_eq!(screen.cursor_x, 16);
    screen.tab_backward(1);
    assert_eq!(screen.cursor_x, 8);
  }

  #[test]
  fn wide_wrapping_at_the_right_edge() {
    let mut screen = Screen::new(4, 3);
    typed(&mut screen, "abc");
    typed(&mut screen, "日");
    assert_eq!(screen.cursor_y, 1, "wide glyph wraps instead of splitting");
    assert_eq!(screen.line(1).unwrap().cell(0).unwrap().ch, '日');
  }

  #[test]
  fn resize_keeps_content() {
    let mut screen = screen();
    typed(&mut screen, "keep me");
    screen.resize(10, 10);
    assert_eq!(screen.line(0).unwrap().text(), "keep me");
    assert_eq!(screen.cols, 10);
    assert_eq!(screen.rows, 10);
  }

  #[test]
  fn selection_view_maps_to_virtual_lines() {
    let mut screen = screen();
    for row in 0..10 {
      for ch in format!("row{row}").chars() {
        screen.print(ch);
      }
      screen.carriage_return();
      screen.line_feed();
    }
    assert!(screen.history_len() > 0, "output scrolled into the history");
    let top = screen.view_top();
    assert_eq!(top, screen.total_lines() - screen.rows as usize);
    assert!(screen.line(top).is_some(), "the top view line exists");
    assert_eq!(screen.line(screen.cursor_line()).unwrap().text(), "row9");
    // Scrolling back walks up through the history.
    screen.scroll_by(2);
    assert_eq!(screen.view_top(), top - 2);
    screen.scroll_to_bottom();
    assert_eq!(screen.view_top(), top);
  }

  #[test]
  fn special_graphics_translate() {
    assert_eq!(translate('q', 1), '\u{2500}');
    assert_eq!(translate('l', 1), '\u{250c}');
    assert_eq!(translate('q', 0), 'q');
  }

  #[test]
  fn char_widths() {
    assert_eq!(char_width('a'), 1);
    assert_eq!(char_width('日'), 2);
    assert_eq!(char_width('\u{0301}'), 0);
  }

  #[test]
  fn word_selection_helpers() {
    let mut screen = screen();
    typed(&mut screen, "ls -la /home");
    let line = screen.line(0).unwrap();
    // `-la` is one word, the space before it ends it.
    assert_eq!(line.word_start(5), 3, "-la counts as one word");
    assert_eq!(line.word_end(4), 6);
    assert_eq!(line.word_start(8), 7, "paths start at the slash");
  }
}