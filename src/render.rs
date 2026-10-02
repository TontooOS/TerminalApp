//! Grid rendering: draws the terminal screen with TontooUI on Vello.
//!
//! One CoreText layout per style run is cached, so a redraw (cursor
//! blink, theme change, scroll) only re-records the scene instead of
//! reshaping text. The cell size comes from the resolved font, which
//! keeps the grid aligned with whatever monospace face the system
//! provides.

use std::collections::HashMap;

use crate::TontooUI::Color;
use crate::TontooUI::kurbo::{Affine, Line, Point, Rect, Stroke};
use crate::TontooUI::peniko::{Brush, Fill};
use crate::TontooUI::renderer::text::{CTFrame, FontSystem, RichSpan, draw_layout};
use crate::TontooUI::Scene;

use crate::config;
use crate::grid::{Cell, CursorShape, Line as GridLine, Screen, flags};
use crate::theme;

/// Selected cells, addressed by virtual line index and column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
  anchor: (usize, u16),
  head: (usize, u16),
}

impl Selection {
  pub fn new(anchor: (usize, u16), head: (usize, u16)) -> Self {
    Self { anchor, head }
  }

  /// Selection ordered from top left to bottom right.
  pub fn ordered(&self) -> ((usize, u16), (usize, u16)) {
    if self.anchor <= self.head {
      (self.anchor, self.head)
    } else {
      (self.head, self.anchor)
    }
  }

  /// Whether the cell is inside the selection. The head cell counts
  /// only when it sits past the start column, so a plain click selects
  /// nothing.
  pub fn contains(&self, line: usize, column: u16) -> bool {
    let (start, end) = self.ordered();
    if line < start.0 || line > end.0 {
      return false;
    }
    if start.0 == end.0 {
      return column >= start.1 && column < end.1;
    }
    if line == start.0 {
      return column >= start.1;
    }
    if line == end.0 {
      return column < end.1;
    }
    true
  }

  /// Selected text with hard wrapped lines kept as line breaks.
  pub fn text(&self, screen: &Screen) -> String {
    let (start, end) = self.ordered();
    let mut out = String::new();
    for line in start.0..=end.0 {
      let Some(row) = screen.line(line) else {
        continue;
      };
      let from = if line == start.0 { start.1 } else { 0 };
      let to = if line == end.0 { end.1 } else { screen.cols };
      let chunk = match from == 0 && to >= screen.cols {
        true => row.text(),
        false => row.text_range(from, to),
      };
      out.push_str(chunk.trim_end_matches(' '));
      if line < end.0 {
        out.push('\n');
      }
    }
    out
  }
}

/// Cache key of one styled text run.
#[derive(Clone, PartialEq, Eq, Hash)]
struct LayoutKey {
  text: String,
  color: [u8; 4],
  bold: bool,
}

/// Terminal grid area in logical px.
#[derive(Clone, Copy, Debug)]
pub struct Area {
  pub x: f32,
  pub y: f32,
  pub width: f32,
  pub height: f32,
}

/// Upper bound for cached runs before the cache is dropped.
const MAX_CACHED_RUNS: usize = 4096;

/// Screen renderer with a text layout cache.
pub struct GridRenderer {
  font_size: f32,
  cell_width: f32,
  row_height: f32,
  measured_scale: f32,
  layouts: HashMap<LayoutKey, CTFrame>,
}

impl GridRenderer {
  pub fn new() -> Self {
    let font_size = config::font_size();
    Self {
      font_size,
      cell_width: font_size * 0.6,
      row_height: font_size * 1.25,
      measured_scale: 0.0,
      layouts: HashMap::new(),
    }
  }

  /// Measure the resolved monospace face. Redone whenever the window
  /// scale changes, since CoreText builds layouts in physical px.
  ///
  /// The grid always asks for the monospace generic family, never for a
  /// family by name: a name list like `"SF Mono", monospace` is parsed
  /// as one family and silently falls back to the proportional system
  /// font, which would break the column alignment.
  pub fn measure(&mut self, fonts: &mut FontSystem) {
    if self.measured_scale == fonts.scale && self.measured_scale > 0.0 {
      return;
    }
    let probe = "M".repeat(40);
    let frame = layout_mono(fonts, &probe, self.font_size, Color::WHITE, false);
    let (width, height) = frame.size();
    let advance = width / probe.chars().count() as f32;
    self.cell_width = advance.max(1.0);
    self.row_height = height.max(self.font_size).ceil();
    self.measured_scale = fonts.scale;
    self.layouts.clear();
  }

  /// Cell size in logical px as `(width, height)`.
  #[cfg(test)]
  pub fn cell_size(&self) -> (f32, f32) {
    (self.cell_width, self.row_height)
  }

  /// Grid size that fits `area`.
  pub fn grid_size(&self, area: Area) -> (u16, u16) {
    let cols = (area.width / self.cell_width).floor().max(2.0) as u16;
    let rows = (area.height / self.row_height).floor().max(1.0) as u16;
    (cols, rows)
  }

  /// Grid cell at a logical point, or `None` outside the area.
  pub fn cell_at(&self, area: Area, x: f32, y: f32) -> Option<(u16, u16)> {
    if x < area.x
      || y < area.y
      || x >= area.x + area.width
      || y >= area.y + area.height
    {
      return None;
    }
    let column = ((x - area.x) / self.cell_width).floor().max(0.0) as u16;
    let row = ((y - area.y) / self.row_height).floor().max(0.0) as u16;
    Some((column, row))
  }

  /// Logical rect of one cell.
  pub fn cell_rect(&self, area: Area, row: u16, column: u16) -> Rect {
    let x = area.x + column as f32 * self.cell_width;
    let y = area.y + row as f32 * self.row_height;
    Rect::new(
      x as f64,
      y as f64,
      x as f64 + self.cell_width as f64,
      y as f64 + self.row_height as f64,
    )
  }

  /// Draw the visible part of `screen`.
  pub fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    screen: &Screen,
    area: Area,
    dark: bool,
    selection: Option<Selection>,
    focused: bool,
    time_secs: f64,
  ) {
    let (cols, rows) = self.grid_size(area);
    let body = physical(area.x, area.y, area.width, area.height, fonts.scale);
    scene.fill(
      Fill::NonZero,
      Affine::IDENTITY,
      &Brush::Solid(theme::background(dark)),
      None,
      &body,
    );

    let top = screen.view_top();
    for row in 0..rows {
      let line_index = top + row as usize;
      let Some(line) = screen.line(line_index) else {
        continue;
      };
      let line = line.clone();
      self.draw_row(
        scene,
        fonts,
        &line,
        line_index,
        row,
        area,
        dark,
        selection,
        focused,
        time_secs,
        cols,
      );
    }

    self.draw_cursor(scene, fonts, screen, area, dark, focused, time_secs, rows);
  }

  #[allow(clippy::too_many_arguments)]
  fn draw_row(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    line: &GridLine,
    line_index: usize,
    row: u16,
    area: Area,
    dark: bool,
    selection: Option<Selection>,
    focused: bool,
    time_secs: f64,
    cols: u16,
  ) {
    let scale = fonts.scale;
    let cells = line.cells();
    let mut runs: Vec<(u16, u16)> = Vec::new();
    let mut column = 0u16;
    while column < cols {
      let Some(cell) = cells.get(column as usize) else {
        break;
      };
      if cell.width == 0 {
        column += 1;
        continue;
      }
      let end = (column + run_length(cells, column, cols)).min(cols);
      runs.push((column, end));
      column = end;
    }

    for &(from, _to) in &runs {
      let attrs = cells[from as usize].attrs;
      let end = from + segment_end(cells, from, cols, selection, line_index);
      let selected = selection
        .map(|selection| selection.contains(line_index, from))
        .unwrap_or(false);
      self.fill_run(scene, attrs, dark, selected, row, area, from, end, scale);
    }

    for &(from, _run_end) in &runs {
      let attrs = cells[from as usize].attrs;
      let mut text = String::new();
      let end = from + run_length(cells, from, cols);
      for index in from..end {
        if let Some(cell) = cells.get(index as usize) {
          if cell.width != 0 {
            text.push(cell.ch);
          }
        }
      }
      self.draw_run_text(
        scene,
        fonts,
        &text,
        attrs,
        dark,
        row,
        area,
        from,
        end,
        scale,
        time_secs,
      );
    }
    let _ = focused;
  }

  #[allow(clippy::too_many_arguments)]
  fn fill_run(
    &mut self,
    scene: &mut Scene,
    attrs: crate::grid::Attrs,
    dark: bool,
    selected: bool,
    row: u16,
    area: Area,
    from: u16,
    to: u16,
    scale: f32,
  ) {
    let inverse = attrs.is_set(flags::INVERSE);
    let own = theme::resolve_background(attrs.bg, dark);
    let fill = match (own, inverse) {
      (Some(color), false) => Some(color),
      (None, true) => Some(theme::foreground(dark)),
      (Some(_), true) => Some(theme::background(dark)),
      (None, false) if selected => Some(theme::selection_fill(dark)),
      (None, false) => None,
    };
    let Some(fill) = fill else {
      return;
    };
    let rect = self.run_rect(row, area, from, to, scale);
    scene.fill(
      Fill::NonZero,
      Affine::IDENTITY,
      &Brush::Solid(fill),
      None,
      &rect,
    );
  }

  #[allow(clippy::too_many_arguments)]
  fn draw_run_text(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    text: &str,
    attrs: crate::grid::Attrs,
    dark: bool,
    row: u16,
    area: Area,
    from: u16,
    to: u16,
    scale: f32,
    time_secs: f64,
  ) {
    let inverse = attrs.is_set(flags::INVERSE);
    let fg = if inverse {
      theme::resolve_background(attrs.bg, dark).unwrap_or_else(|| theme::background(dark))
    } else {
      theme::resolve(attrs.fg, dark)
    };
    let blink_hidden = attrs.is_set(flags::BLINK) && !blink_on(time_secs);
    let x = area.x + from as f32 * self.cell_width;
    let y = area.y + row as f32 * self.row_height;
    let width = (to - from) as f32 * self.cell_width;

    if !attrs.is_set(flags::HIDDEN) && !blink_hidden && !text.trim().is_empty() {
      let mut color = fg;
      if attrs.is_set(flags::DIM) {
        let rgba = color.to_rgba8();
        let blend = |value: u8| (u16::from(value) * 3 / 5) as u8;
        color = Color::from_rgba8(blend(rgba.r), blend(rgba.g), blend(rgba.b), rgba.a);
      }
      let bold = attrs.is_set(flags::BOLD);
      let key = layout_key(text, color, bold);
      self.ensure_frame(fonts, &key, color, bold);
      if let Some(frame) = self.layouts.get(&key) {
        draw_layout(scene, frame, x, y, scale);
      }
    }

    if attrs.is_set(flags::UNDERLINE) {
      let thickness = (scale as f64).max(1.0);
      let line = Line::new(
        Point::new(
          x as f64 * scale as f64,
          (y + self.row_height - 2.0) as f64 * scale as f64,
        ),
        Point::new(
          (x + width) as f64 * scale as f64,
          (y + self.row_height - 2.0) as f64 * scale as f64,
        ),
      );
      scene.stroke(
        &Stroke::new(thickness),
        Affine::IDENTITY,
        &Brush::Solid(fg),
        None,
        &line,
      );
    }

    if attrs.is_set(flags::STRIKE) {
      let line = Line::new(
        Point::new(
          x as f64 * scale as f64,
          (y + self.row_height / 2.0) as f64 * scale as f64,
        ),
        Point::new(
          (x + width) as f64 * scale as f64,
          (y + self.row_height / 2.0) as f64 * scale as f64,
        ),
      );
      scene.stroke(
        &Stroke::new((scale as f64).max(1.0)),
        Affine::IDENTITY,
        &Brush::Solid(fg),
        None,
        &line,
      );
    }
  }

  /// Cached CoreText layout of one run, built on first use.
  fn ensure_frame(
    &mut self,
    fonts: &mut FontSystem,
    key: &LayoutKey,
    color: Color,
    bold: bool,
  ) {
    if self.layouts.len() > MAX_CACHED_RUNS {
      self.layouts.clear();
    }
    if self.layouts.contains_key(key) {
      return;
    }
    let frame = layout_mono(fonts, &key.text, self.font_size, color, bold);
    self.layouts.insert(key.clone(), frame);
  }

  fn run_rect(&self, row: u16, area: Area, from: u16, to: u16, scale: f32) -> Rect {
    physical(
      area.x + from as f32 * self.cell_width,
      area.y + row as f32 * self.row_height,
      (to - from) as f32 * self.cell_width,
      self.row_height,
      scale,
    )
  }

  fn draw_cursor(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    screen: &Screen,
    area: Area,
    dark: bool,
    focused: bool,
    time_secs: f64,
    rows: u16,
  ) {
    if !screen.modes.cursor_visible {
      return;
    }
    let top = screen.view_top();
    let cursor_line = screen.cursor_line();
    if cursor_line < top || cursor_line >= top + rows as usize {
      return;
    }
    let row = (cursor_line - top) as u16;
    let column = screen.cursor_x.min(screen.cols.saturating_sub(1));
    let scale = fonts.scale;
    let logical = self.cell_rect(area, row, column);
    let cell = physical(
      logical.x0 as f32,
      logical.y0 as f32,
      logical.width() as f32,
      logical.height() as f32,
      scale,
    );
    let fill = theme::cursor_fill(dark);
    match screen.shape {
      CursorShape::Block => {
        if focused {
          // Static block: a blinking cursor flipped the glyph under it
          // between the text color and the background color every
          // cycle, which read as flickering text.
          scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Brush::Solid(fill),
            None,
            &cell,
          );
          self.draw_cursor_glyph(scene, fonts, screen, area, dark, scale, time_secs);
        } else {
          scene.stroke(
            &Stroke::new((scale as f64).max(1.0)),
            Affine::IDENTITY,
            &Brush::Solid(fill),
            None,
            &cell,
          );
        }
      }
      CursorShape::Underline | CursorShape::Bar => {
        let thickness = (2.0 * scale as f64).max(1.0);
        let bar = match screen.shape {
          CursorShape::Underline => Rect::new(
            cell.x0,
            cell.y1 - thickness,
            cell.width(),
            thickness,
          ),
          _ => Rect::new(
            cell.x0,
            cell.y0 + (cell.height() - thickness) / 2.0,
            thickness,
            cell.height(),
          ),
        };
        scene.fill(
          Fill::NonZero,
          Affine::IDENTITY,
          &Brush::Solid(fill),
          None,
          &bar,
        );
      }
    }
  }

  /// Glyph under a filled block cursor, painted in the background
  /// color so it reads as an inverted cell.
  fn draw_cursor_glyph(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    screen: &Screen,
    area: Area,
    dark: bool,
    scale: f32,
    time_secs: f64,
  ) {
    let line_index = screen.cursor_line();
    let Some(line) = screen.line(line_index) else {
      return;
    };
    let Some(cell) = line.cell(screen.cursor_x) else {
      return;
    };
    if cell.ch == ' ' || cell.ch == '\0' || cell.attrs.is_set(flags::HIDDEN) {
      return;
    }
    if cell.attrs.is_set(flags::BLINK) && !blink_on(time_secs) {
      return;
    }
    let color = if cell.attrs.is_set(flags::INVERSE) {
      theme::resolve(cell.attrs.fg, dark)
    } else {
      theme::background(dark)
    };
    let bold = cell.attrs.is_set(flags::BOLD);
    let key = layout_key(&cell.ch.to_string(), color, bold);
    self.ensure_frame(fonts, &key, color, bold);
    let top = screen.view_top();
    let row = line_index.saturating_sub(top) as u16;
    let column = screen.cursor_x;
    if let Some(frame) = self.layouts.get(&key) {
      draw_layout(
        scene,
        frame,
        area.x + column as f32 * self.cell_width,
        area.y + row as f32 * self.row_height,
        scale,
      );
    }
  }
}

/// Cache key of one styled text run.
fn layout_key(text: &str, color: Color, bold: bool) -> LayoutKey {
  let rgba = color.to_rgba8();
  LayoutKey {
    text: text.to_string(),
    color: [rgba.r, rgba.g, rgba.b, rgba.a],
    bold,
  }
}

/// Lay out grid text in the monospace generic family.
///
/// CoreText pushes a quoted family stack per family name, so naming a
/// list (`SF Mono, monospace`) resolves to a single missing family and
/// falls back to the proportional system font. The generic monospace
/// family always resolves to the real monospace face, which keeps every
/// glyph on the same advance.
fn layout_mono(
  fonts: &mut FontSystem,
  text: &str,
  size: f32,
  color: Color,
  bold: bool,
) -> CTFrame {
  let span = RichSpan {
    range: 0..text.len(),
    monospace: true,
    bold,
    ..RichSpan::default()
  };
  fonts.layout_rich_text(text, size, color, None, &[span])
}

/// Logical rect to physical scene px.
fn physical(x: f32, y: f32, width: f32, height: f32, scale: f32) -> Rect {
  let factor = scale as f64;
  let x = x as f64 * factor;
  let y = y as f64 * factor;
  Rect::new(x, y, x + width as f64 * factor, y + height as f64 * factor)
}

/// Length in cells of the run starting at `column` with an equal style.
fn run_length(cells: &[Cell], column: u16, cols: u16) -> u16 {
  let start = cells[column as usize].attrs;
  let mut length = 0u16;
  let mut index = column as usize;
  while index < cells.len() && (index as u16) < cols {
    let cell = &cells[index];
    if cell.attrs != start {
      break;
    }
    length += u16::from(cell.width.max(1));
    index += 1;
  }
  length.max(1)
}

/// Length of the background segment starting at `from`: a run ends
/// early where the selection starts or ends, so a selection inside a
/// long run (a whole prompt line) still gets its own fill.
fn segment_end(
  cells: &[Cell],
  from: u16,
  cols: u16,
  selection: Option<Selection>,
  line: usize,
) -> u16 {
  let length = run_length(cells, from, cols);
  let Some(selection) = selection else {
    return length;
  };
  let inside = selection.contains(line, from);
  for offset in 1..=length {
    let column = from + offset;
    if column >= cols {
      return column - from;
    }
    if selection.contains(line, column) != inside {
      return offset;
    }
  }
  length
}

/// Blink phase for SGR 5 and 6 text: on for the first half of the period.
fn blink_on(time_secs: f64) -> bool {
  let phase = time_secs / config::BLINK_SECONDS;
  (phase - phase.floor()) < 0.5
}

#[cfg(test)]
mod tests {
  use super::*;

  fn area() -> Area {
    Area {
      x: 0.0,
      y: 0.0,
      width: 800.0,
      height: 500.0,
    }
  }

  fn renderer() -> GridRenderer {
    let mut renderer = GridRenderer::new();
    renderer.cell_width = 8.0;
    renderer.row_height = 18.0;
    renderer
  }

  #[test]
  fn grid_size_fits_the_area() {
    assert_eq!(renderer().grid_size(area()), (100, 27));
  }

  #[test]
  fn cell_lookup_maps_points() {
    let renderer = renderer();
    assert_eq!(renderer.cell_at(area(), 0.0, 0.0), Some((0, 0)));
    assert_eq!(renderer.cell_at(area(), 8.5, 19.0), Some((1, 1)));
    assert_eq!(renderer.cell_at(area(), -1.0, 0.0), None);
    assert_eq!(renderer.cell_at(area(), 800.0, 0.0), None);
  }

  #[test]
  fn selection_orders_its_ends() {
    let selection = Selection::new((3, 5), (1, 2));
    assert_eq!(selection.ordered(), ((1, 2), (3, 5)));
    // Line 2 is fully inside, line 1 starts at the anchor column and
    // line 3 ends before the head column.
    assert!(selection.contains(2, 0));
    assert!(selection.contains(2, 10));
    assert!(selection.contains(1, 2));
    assert!(!selection.contains(1, 1));
    assert!(selection.contains(3, 4));
    assert!(!selection.contains(3, 5));
    assert!(!selection.contains(4, 0));
  }

  #[test]
  fn selection_on_one_line_spans_columns() {
    let selection = Selection::new((0, 2), (0, 6));
    assert!(selection.contains(0, 3));
    assert!(!selection.contains(0, 6));
    let empty = Selection::new((0, 2), (0, 2));
    assert!(!empty.contains(0, 2), "a click selects nothing");
  }

  #[test]
  fn selection_text_joins_wrapped_lines() {
    let mut screen = Screen::new(20, 5);
    for ch in "hello world".chars() {
      screen.print(ch);
    }
    screen.carriage_return();
    screen.index();
    for ch in "second line".chars() {
      screen.print(ch);
    }
    let selection = Selection::new((0, 6), (1, 6));
    assert_eq!(selection.text(&screen), "world\nsecond");
  }

  #[test]
  fn run_length_stops_at_a_style_change() {
    let mut screen = Screen::new(10, 2);
    let mut parser = crate::parser::Parser::new();
    let mut reply = Vec::new();
    parser.feed(b"aaa\x1b[31mbbb", &mut screen, &mut reply, 0.0);
    let line = screen.line(0).unwrap();
    assert_eq!(run_length(line.cells(), 0, 10), 3);
    assert_eq!(run_length(line.cells(), 3, 10), 3);
  }

  #[test]
  fn selection_splits_a_long_run() {
    let mut screen = Screen::new(20, 3);
    let mut parser = crate::parser::Parser::new();
    let mut reply = Vec::new();
    parser.feed(b"hello world", &mut screen, &mut reply, 0.0);
    let cells = screen.line(0).unwrap().cells();
    // One uniform style fills the row, so a run reaches the last column.
    assert_eq!(run_length(cells, 0, 20), 20);
    assert_eq!(segment_end(cells, 0, 20, None, 0), 20);
    // A selection inside the run must cut the segment, otherwise the
    // selection never gets a fill.
    let selection = Selection::new((0, 6), (0, 11));
    assert_eq!(segment_end(cells, 0, 20, Some(selection), 0), 6);
    assert_eq!(segment_end(cells, 6, 20, Some(selection), 0), 5);
    assert_eq!(segment_end(cells, 11, 20, Some(selection), 0), 9);
    // A selection starting at the run start covers all of it.
    let all = Selection::new((0, 0), (0, 11));
    assert_eq!(segment_end(cells, 0, 20, Some(all), 0), 11);
    assert_eq!(segment_end(cells, 11, 20, Some(all), 0), 9);
  }

  #[test]
  fn blink_phase_alternates() {
    assert!(blink_on(0.0));
    assert!(!blink_on(config::BLINK_SECONDS * 0.75));
    assert!(blink_on(config::BLINK_SECONDS * 1.1));
  }

  #[test]
  fn mono_layout_gives_every_glyph_the_same_advance() {
    let mut fonts = FontSystem::new();
    let sample = "MWiml.@#";
    let (wide, _) = layout_mono(&mut fonts, sample, config::FONT_SIZE, Color::WHITE, false)
      .size();
    let (single, _) =
      layout_mono(&mut fonts, "M", config::FONT_SIZE, Color::WHITE, false).size();
    let per_char = wide / sample.chars().count() as f32;
    assert!(
      (per_char - single).abs() < 0.5,
      "proportional font leaked in: {per_char} per char vs {single} for M"
    );
    assert!(
      (per_char / config::FONT_SIZE - 0.6).abs() < 0.2,
      "advance {per_char} is not a monospace width at {}",
      config::FONT_SIZE
    );
  }

  #[test]
  fn cell_rect_uses_the_cell_size() {
    let rect = renderer().cell_rect(area(), 2, 3);
    assert_eq!((rect.x0, rect.y0), (24.0, 36.0));
    assert_eq!((rect.width(), rect.height()), (8.0, 18.0));
  }
}