//! VT parser: the state machine that turns program output into screen
//! operations.
//!
//! Implements the VT100 / xterm subset a modern shell and full screen
//! programs need: C0 controls, `ESC` sequences, CSI and OSC strings,
//! SGR colors (16, 256, direct RGB), DECSET / DECRST modes including the
//! alternate screen and mouse reporting, DEC Special Graphics and the
//! device status replies a shell expects. Bytes that arrive split
//! across reads are buffered, so a sequence may span chunks.

use crate::grid::{flags, Attrs, Color, CursorShape, Screen};

/// Upper bound for a buffered OSC or DCS string, so a program cannot
/// grow the buffer without end.
const MAX_STRING: usize = 8192;

/// Upper bound for buffered parameters of one sequence.
const MAX_PARAMS: usize = 32;

/// Parser state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
  #[default]
  Ground,
  Escape,
  EscapeIntermediate,
  CsiParam,
  CsiIntermediate,
  OscString,
  DcsEntry,
  DcsPassthrough,
  DcsParam,
  SosPmApcString,
}

/// The VT state machine.
#[derive(Debug, Default)]
pub struct Parser {
  state: State,
  params: Vec<u16>,
  param_pending: bool,
  private: bool,
  intermediates: Vec<u8>,
  string: Vec<u8>,
  utf8: Vec<u8>,
  /// Character of the last REP sequence (CSI b).
  repeat: char,
}

impl Parser {
  pub fn new() -> Self {
    Self {
      state: State::Ground,
      params: Vec::new(),
      param_pending: false,
      private: false,
      intermediates: Vec::new(),
      string: Vec::new(),
      utf8: Vec::new(),
      repeat: ' ',
    }
  }

  /// A complete reset (RIS) also resets the parser itself.
  pub fn reset(&mut self) {
    *self = Self::new();
  }

  /// Feed program output. `reply` collects bytes the terminal sends
  /// back (device reports, color queries).
  pub fn feed(&mut self, bytes: &[u8], screen: &mut Screen, reply: &mut Vec<u8>, now_secs: f64) {
    for &byte in bytes {
      self.step(byte, screen, reply, now_secs);
    }
  }

  fn step(&mut self, byte: u8, screen: &mut Screen, reply: &mut Vec<u8>, now_secs: f64) {
    match self.state {
      State::Ground => self.ground(byte, screen, reply, now_secs),
      State::Escape => self.escape(byte, screen, reply),
      State::EscapeIntermediate => self.escape_intermediate(byte, screen),
      State::CsiParam => self.csi_param(byte, screen, reply),
      State::CsiIntermediate => self.csi_intermediate(byte, screen, reply),
      State::OscString => self.osc(byte, screen, reply),
      State::DcsEntry => self.dcs_entry(byte),
      State::DcsPassthrough | State::DcsParam | State::SosPmApcString => self.dcs_data(byte),
    }
  }

  fn ground(&mut self, byte: u8, screen: &mut Screen, _reply: &mut Vec<u8>, now_secs: f64) {
    match byte {
      0x00..=0x06 | 0x10..=0x17 | 0x19 | 0x1c..=0x1f => {}
      0x07 => screen.bell(now_secs),
      0x08 => screen.cursor_back(1),
      0x09 => screen.tab_forward(1),
      0x0a | 0x0b | 0x0c => screen.line_feed(),
      0x0d => screen.carriage_return(),
      0x0e => screen.invoke(true),
      0x0f => screen.invoke(false),
      0x18 | 0x1a => {}
      0x1b => {
        self.begin_sequence();
        self.state = State::Escape;
      }
      0x90 => {
        self.begin_sequence();
        self.state = State::SosPmApcString;
      }
      0x7f => {}
      _ => self.print(byte, screen),
    }
  }

  fn begin_sequence(&mut self) {
    self.params.clear();
    self.param_pending = false;
    self.private = false;
    self.intermediates.clear();
    self.string.clear();
  }

  fn print(&mut self, byte: u8, screen: &mut Screen) {
    if !self.utf8.is_empty() {
      if let Some(ch) = decode_utf8(&mut self.utf8, byte) {
        self.repeat = ch;
        screen.print(ch);
      }
      return;
    }
    if byte < 0x80 {
      let ch = byte as char;
      self.repeat = ch;
      screen.print(ch);
      return;
    }
    if byte & 0xc0 == 0x80 {
      return;
    }
    self.utf8.clear();
    self.utf8.push(byte);
  }

  fn escape(&mut self, byte: u8, screen: &mut Screen, reply: &mut Vec<u8>) {
    match byte {
      b'[' => self.state = State::CsiParam,
      b']' => {
        self.string.clear();
        self.state = State::OscString;
      }
      b'P' => {
        self.string.clear();
        self.state = State::DcsEntry;
      }
      b'X' | b'^' | b'_' => {
        self.string.clear();
        self.state = State::SosPmApcString;
      }
      0x20..=0x2f => {
        if self.intermediates.len() < 4 {
          self.intermediates.push(byte);
        }
        self.state = State::EscapeIntermediate;
      }
      0x1b => self.state = State::Escape,
      _ => {
        self.state = State::Ground;
        self.esc_dispatch(byte, screen, reply);
      }
    }
  }

  fn escape_intermediate(&mut self, byte: u8, screen: &mut Screen) {
    match byte {
      0x20..=0x2f => {
        if self.intermediates.len() < 4 {
          self.intermediates.push(byte);
        }
      }
      0x1b => self.state = State::Escape,
      _ => {
        self.state = State::Ground;
        match self.intermediates.first().copied() {
          // DECALN: fill the grid for alignment tests.
          Some(b'#') if byte == b'8' => screen.fill_rectangle('E'),
          Some(b'(') => screen.designate(0, charset_id(byte)),
          Some(b')') | Some(b'-') => screen.designate(1, charset_id(byte)),
          _ => {}
        }
      }
    }
  }

  fn csi_param(&mut self, byte: u8, screen: &mut Screen, reply: &mut Vec<u8>) {
    match byte {
      b'0'..=b'9' => {
        if self.params.is_empty() {
          self.params.push(0);
        }
        let last = self.params.len() - 1;
        let slot = &mut self.params[last];
        *slot = slot.saturating_mul(10).min(u16::MAX - 10) + (byte - b'0') as u16;
        self.param_pending = true;
      }
      b';' => {
        if !self.param_pending {
          self.params.push(0);
        }
        if self.params.len() < MAX_PARAMS {
          self.params.push(0);
        }
        self.param_pending = false;
      }
      b':' => {}
      b'<' | b'=' | b'>' | b'?' => self.private = true,
      b' '..=b'/' => {
        if self.intermediates.len() < 4 {
          self.intermediates.push(byte);
        }
        self.state = State::CsiIntermediate;
      }
      0x40..=0x7e => {
        self.state = State::Ground;
        self.csi_dispatch(byte, screen, reply);
      }
      0x1b => {
        self.begin_sequence();
        self.state = State::Escape;
      }
      _ => self.state = State::Ground,
    }
  }

  fn csi_intermediate(&mut self, byte: u8, screen: &mut Screen, reply: &mut Vec<u8>) {
    match byte {
      0x20..=0x2f => {
        if self.intermediates.len() < 4 {
          self.intermediates.push(byte);
        }
      }
      0x40..=0x7e => {
        self.state = State::Ground;
        self.csi_dispatch(byte, screen, reply);
      }
      0x1b => {
        self.begin_sequence();
        self.state = State::Escape;
      }
      _ => self.state = State::Ground,
    }
  }

  fn osc(&mut self, byte: u8, screen: &mut Screen, reply: &mut Vec<u8>) {
    match byte {
      0x07 => {
        self.osc_dispatch(screen, reply);
        self.state = State::Ground;
      }
      0x1b => {
        self.osc_dispatch(screen, reply);
        self.begin_sequence();
        self.state = State::Escape;
      }
      _ => {
        if self.string.len() < MAX_STRING {
          self.string.push(byte);
        }
      }
    }
  }

  fn dcs_entry(&mut self, byte: u8) {
    match byte {
      0x1b => self.state = State::Escape,
      b'0'..=b'9' | b';' => self.state = State::DcsParam,
      _ => self.state = State::DcsPassthrough,
    }
  }

  fn dcs_data(&mut self, byte: u8) {
    match byte {
      0x1b => self.state = State::Escape,
      _ => {
        if self.string.len() < MAX_STRING {
          self.string.push(byte);
        }
      }
    }
  }

  /// Parameter `index`, 1-based, with `default` for missing or zero
  /// values.
  fn param(&self, index: usize, default: u64) -> u64 {
    match self.params.get(index) {
      Some(0) | None => default,
      Some(value) => *value as u64,
    }
  }

  fn param_raw(&self, index: usize) -> u64 {
    self.params.get(index).copied().unwrap_or(0) as u64
  }

  fn esc_dispatch(&mut self, byte: u8, screen: &mut Screen, _reply: &mut Vec<u8>) {
    match byte {
      b'7' => screen.save_cursor(),
      b'8' => screen.restore_cursor(),
      b'D' => screen.index(),
      b'M' => screen.reverse_index(),
      b'E' => {
        screen.carriage_return();
        screen.index();
      }
      b'H' => screen.set_tab(),
      b'c' => {
        screen.reset();
        self.reset();
      }
      b'=' => screen.modes.app_keypad = true,
      b'>' => screen.modes.app_keypad = false,
      b'\\' | b'Z' => {}
      _ => {}
    }
  }

  fn csi_dispatch(&mut self, final_byte: u8, screen: &mut Screen, reply: &mut Vec<u8>) {
    match self.intermediates.first().copied() {
      Some(b' ') => {
        if final_byte == b'q' {
          screen.shape = CursorShape::from_decscusr(self.param(0, 1));
        }
        return;
      }
      Some(b'#') => {
        if final_byte == b'8' {
          screen.fill_rectangle('E');
        }
        return;
      }
      Some(_) => return,
      None => {}
    }
    match final_byte {
      b'@' => {
        let count = self.param(0, 1) as u16;
        let (row, col) = (screen.cursor_y, screen.cursor_x);
        screen.insert_blank(row, col, count);
      }
      b'A' => screen.cursor_up(self.param(0, 1) as u16),
      b'B' | b'e' => screen.cursor_down(self.param(0, 1) as u16),
      b'C' | b'a' => screen.cursor_forward(self.param(0, 1) as u16),
      b'D' => screen.cursor_back(self.param(0, 1) as u16),
      b'E' => {
        screen.cursor_down(self.param(0, 1) as u16);
        screen.carriage_return();
      }
      b'F' => {
        screen.cursor_up(self.param(0, 1) as u16);
        screen.carriage_return();
      }
      b'G' | b'`' => screen.move_to_column(self.param(0, 1).saturating_sub(1) as u16),
      b'H' | b'f' => {
        let row = self.param(0, 1) as u16;
        let col = self.param(1, 1) as u16;
        screen.goto(row.saturating_sub(1), col.saturating_sub(1));
      }
      b'I' => screen.tab_forward(self.param(0, 1) as u16),
      b'J' => screen.erase_in_display(self.param(0, 0)),
      b'K' => screen.erase_in_line(self.param(0, 0)),
      b'L' => screen.insert_lines(self.param(0, 1) as u16),
      b'M' => screen.delete_lines(self.param(0, 1) as u16),
      b'P' => {
        let count = self.param(0, 1) as u16;
        let (row, col) = (screen.cursor_y, screen.cursor_x);
        screen.delete_chars(row, col, count);
      }
      b'S' => screen.scroll_up(self.param(0, 1) as u16),
      b'T' => screen.scroll_down(self.param(0, 1) as u16),
      b'X' => screen.erase_chars(self.param(0, 1) as u16),
      b'Z' => screen.tab_backward(self.param(0, 1) as u16),
      b'b' => {
        let count = self.param(0, 1);
        let ch = self.repeat;
        for _ in 0..count {
          screen.print(ch);
        }
      }
      b'd' => screen.move_to_row(self.param(0, 1).saturating_sub(1) as u16),
      b'g' => screen.clear_tabs(self.param(0, 0)),
      b'h' => self.set_modes(screen, false),
      b'l' => self.set_modes(screen, true),
      b'm' => self.sgr(screen),
      b'n' => self.device_status(screen, reply),
      b'c' => reply.extend_from_slice(b"\x1b[?1;2c"),
      b'r' => {
        let rows = screen.rows as u64;
        let top = self.param(0, 1) as u16;
        let bottom = self.param(1, rows) as u16;
        screen.set_scroll_region(top.saturating_sub(1), bottom.saturating_sub(1));
      }
      b's' => screen.save_cursor(),
      b'u' => screen.restore_cursor(),
      _ => {}
    }
  }

  fn device_status(&mut self, screen: &mut Screen, reply: &mut Vec<u8>) {
    match self.param(0, 0) {
      5 => reply.extend_from_slice(b"\x1b[0n"),
      6 => {
        let report = screen.cursor_position_report();
        reply.extend_from_slice(b"\x1b[");
        reply.extend_from_slice(report.as_bytes());
        reply.extend_from_slice(b"R");
      }
      _ => {}
    }
  }

  /// SM (`CSI h`) and RM (`CSI l`), both the ANSI and the DEC private
  /// form.
  fn set_modes(&mut self, screen: &mut Screen, reset: bool) {
    let on = !reset;
    for index in 0..self.params.len() {
      let mode = self.param_raw(index);
      if self.private {
        self.apply_dec_mode(screen, mode, on);
      } else {
        self.apply_ansi_mode(screen, mode, on);
      }
    }
  }

  fn apply_ansi_mode(&mut self, screen: &mut Screen, mode: u64, on: bool) {
    match mode {
      4 => screen.modes.insert = on,
      20 => screen.modes.newline = on,
      _ => {}
    }
  }

  fn apply_dec_mode(&mut self, screen: &mut Screen, mode: u64, on: bool) {
    match mode {
      1 => screen.modes.app_cursor_keys = on,
      5 => screen.modes.reverse_wrap = on,
      6 => {
        screen.modes.origin = on;
        screen.goto_home();
      }
      7 => screen.modes.autowrap = on,
      8 => screen.modes.reverse_wrap = on,
      9 => screen.modes.mouse_x10 = on,
      25 => screen.modes.cursor_visible = on,
      47 | 1047 | 1049 => {
        let clear = on && mode != 47;
        if on {
          screen.save_cursor();
        }
        screen.set_alt_screen(on, clear);
        if !on {
          screen.restore_cursor();
        }
      }
      1048 => {
        if on {
          screen.save_cursor();
        } else {
          screen.restore_cursor();
        }
      }
      1000 => screen.modes.mouse_button = on,
      1002 => screen.modes.mouse_drag = on,
      1003 => screen.modes.mouse_any = on,
      1004 => screen.modes.focus_events = on,
      1006 => screen.modes.mouse_sgr = on,
      2004 => screen.modes.bracketed_paste = on,
      _ => {}
    }
  }

  /// SGR: colors and text attributes.
  fn sgr(&mut self, screen: &mut Screen) {
    if self.params.is_empty() {
      screen.attrs = Attrs::default();
      return;
    }
    let mut index = 0;
    while index < self.params.len() {
      let code = self.param_raw(index);
      match code {
        0 => screen.attrs = Attrs::default(),
        1 => screen.attrs.set(flags::BOLD, true),
        2 => screen.attrs.set(flags::DIM, true),
        3 => screen.attrs.set(flags::ITALIC, true),
        4 => screen.attrs.set(flags::UNDERLINE, true),
        5 | 6 => screen.attrs.set(flags::BLINK, true),
        7 => screen.attrs.set(flags::INVERSE, true),
        8 => screen.attrs.set(flags::HIDDEN, true),
        9 => screen.attrs.set(flags::STRIKE, true),
        21 | 22 => {
          screen.attrs.set(flags::BOLD, false);
          screen.attrs.set(flags::DIM, false);
        }
        23 => screen.attrs.set(flags::ITALIC, false),
        24 => screen.attrs.set(flags::UNDERLINE, false),
        25 => screen.attrs.set(flags::BLINK, false),
        27 => screen.attrs.set(flags::INVERSE, false),
        28 => screen.attrs.set(flags::HIDDEN, false),
        29 => screen.attrs.set(flags::STRIKE, false),
        30..=37 => screen.attrs.fg = Color::Indexed((code - 30) as u8),
        38 => index = self.extended_color(screen, index, true),
        39 => screen.attrs.fg = Color::Default,
        40..=47 => screen.attrs.bg = Color::Indexed((code - 40) as u8),
        48 => index = self.extended_color(screen, index, false),
        49 => screen.attrs.bg = Color::Default,
        90..=97 => screen.attrs.fg = Color::Indexed((code - 82) as u8),
        100..=107 => screen.attrs.bg = Color::Indexed((code - 92) as u8),
        _ => {}
      }
      index += 1;
    }
  }

  /// `38` / `48` with `5;n` (palette) or `2;r;g;b` (direct color).
  /// Returns the index of the last consumed parameter.
  fn extended_color(&mut self, screen: &mut Screen, index: usize, foreground: bool) -> usize {
    match self.param_raw(index + 1) {
      5 => {
        let color = Color::Indexed(self.param_raw(index + 2).min(255) as u8);
        if foreground {
          screen.attrs.fg = color;
        } else {
          screen.attrs.bg = color;
        }
        index + 2
      }
      2 => {
        let r = self.param_raw(index + 2).min(255) as u8;
        let g = self.param_raw(index + 3).min(255) as u8;
        let b = self.param_raw(index + 4).min(255) as u8;
        let color = Color::Rgb(r, g, b);
        if foreground {
          screen.attrs.fg = color;
        } else {
          screen.attrs.bg = color;
        }
        index + 4
      }
      _ => index,
    }
  }

  fn osc_dispatch(&mut self, screen: &mut Screen, reply: &mut Vec<u8>) {
    let raw = std::mem::take(&mut self.string);
    if raw.is_empty() {
      return;
    }
    let text = String::from_utf8_lossy(&raw).to_string();
    let mut parts = text.splitn(2, ';');
    let command = parts.next().unwrap_or("").trim().to_string();
    let payload = parts.next().unwrap_or("").to_string();
    match command.as_str() {
      "0" => {
        screen.icon_title = payload.clone();
        screen.title = payload;
      }
      "1" => screen.icon_title = payload,
      "2" => screen.title = payload,
      "7" => screen.cwd = payload,
      "8" => {
        screen.hyperlink = payload
          .split(';')
          .nth(1)
          .filter(|value| !value.is_empty())
          .map(|value| value.to_string());
      }
      "10" | "11" => {
        let foreground = command == "10";
        let dark = !screen.background_is_light;
        let report = crate::theme::color_report(dark, foreground);
        reply.extend_from_slice(b"\x1b]");
        reply.extend_from_slice(command.as_bytes());
        reply.extend_from_slice(b";");
        reply.extend_from_slice(report.as_bytes());
        reply.extend_from_slice(b"\x1b\\");
      }
      "52" => {
        let data = payload.split(';').nth(1).unwrap_or("");
        if let Some(text) = decode_base64(data) {
          screen.clipboard_write = Some(text);
        }
      }
      _ => {}
    }
  }
}

/// Charset designator final byte to the internal id.
fn charset_id(byte: u8) -> u8 {
  match byte {
    b'0' => 1,
    b'A' => 2,
    _ => 0,
  }
}

/// Decode one UTF-8 continuation byte into `pending`, returning the
/// character when the sequence completes. Incomplete input stays
/// buffered.
fn decode_utf8(pending: &mut Vec<u8>, byte: u8) -> Option<char> {
  if byte & 0xc0 != 0x80 {
    pending.clear();
    return None;
  }
  pending.push(byte);
  let expected = utf8_len(pending[0]);
  if pending.len() < expected {
    return None;
  }
  let buffer = std::mem::take(pending);
  std::str::from_utf8(&buffer)
    .ok()
    .and_then(|text| text.chars().next())
}

/// Total byte length of a UTF-8 sequence from its lead byte.
fn utf8_len(lead: u8) -> usize {
  match lead {
    0xc0..=0xdf => 2,
    0xe0..=0xef => 3,
    0xf0..=0xf7 => 4,
    _ => 1,
  }
}

/// Base64 decode for OSC 52 clipboard payloads (standard alphabet).
fn decode_base64(input: &str) -> Option<String> {
  let mut out: Vec<u8> = Vec::with_capacity(input.len() * 3 / 4);
  let mut buffer: u32 = 0;
  let mut bits = 0;
  for ch in input.bytes() {
    let value = match ch {
      b'A'..=b'Z' => ch - b'A',
      b'a'..=b'z' => ch - b'a' + 26,
      b'0'..=b'9' => ch - b'0' + 52,
      b'+' => 62,
      b'/' => 63,
      b'=' => break,
      _ => continue,
    } as u32;
    buffer = (buffer << 6) | value;
    bits += 6;
    if bits >= 8 {
      bits -= 8;
      out.push((buffer >> bits) as u8);
    }
  }
  String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn run(input: &[u8]) -> (Screen, Vec<u8>) {
    let mut screen = Screen::new(20, 5);
    let mut parser = Parser::new();
    let mut reply = Vec::new();
    parser.feed(input, &mut screen, &mut reply, 0.0);
    (screen, reply)
  }

  fn typed(screen: &Screen) -> String {
    screen
      .line(screen.cursor_line())
      .map(|line| line.text())
      .unwrap_or_default()
  }

  #[test]
  fn plain_text_lands_in_the_grid() {
    let (screen, _) = run(b"hello");
    assert_eq!(typed(&screen), "hello");
    assert_eq!(screen.cursor_x, 5);
  }

  #[test]
  fn cr_lf_and_backspace() {
    let (screen, _) = run(b"abc\r\ndef");
    assert_eq!(typed(&screen), "def");
    assert_eq!(screen.cursor_y, 1);
    let (screen, _) = run(b"abc\x08\x08X");
    assert_eq!(typed(&screen), "aXc");
  }

  #[test]
  fn sequences_survive_split_reads() {
    let mut screen = Screen::new(20, 5);
    let mut parser = Parser::new();
    let mut reply = Vec::new();
    for chunk in [b"\x1b[3".as_slice(), b"1mred".as_slice()] {
      parser.feed(chunk, &mut screen, &mut reply, 0.0);
    }
    assert_eq!(typed(&screen), "red");
    assert_eq!(
      screen.line(0).unwrap().cell(0).unwrap().attrs.fg,
      Color::Indexed(1)
    );
  }

  #[test]
  fn cursor_positioning_and_erase_line() {
    let (screen, _) = run(b"\x1b[2;3Hxy");
    assert_eq!(screen.cursor_x, 4);
    assert_eq!(screen.cursor_y, 1);
    let (screen, _) = run(b"\x1b[2;3H\x1b[K");
    assert_eq!(screen.line(1).unwrap().text(), "");
  }

  #[test]
  fn sgr_colors_and_attributes() {
    let (screen, _) = run(b"\x1b[38;2;18;52;86mx");
    assert_eq!(
      screen.line(0).unwrap().cell(0).unwrap().attrs.fg,
      Color::Rgb(18, 52, 86)
    );
    let (screen, _) = run(b"\x1b[48;5;200m x");
    assert_eq!(
      screen.line(0).unwrap().cell(1).unwrap().attrs.bg,
      Color::Indexed(200)
    );
    let (screen, _) = run(b"\x1b[1;4;7mx");
    let attrs = screen.line(0).unwrap().cell(0).unwrap().attrs;
    assert!(attrs.is_set(flags::BOLD));
    assert!(attrs.is_set(flags::UNDERLINE));
    assert!(attrs.is_set(flags::INVERSE));
    let (screen, _) = run(b"\x1b[91mx");
    assert_eq!(
      screen.line(0).unwrap().cell(0).unwrap().attrs.fg,
      Color::Indexed(9)
    );
  }

  #[test]
  fn osc_title_and_cwd() {
    let (screen, _) = run(b"\x1b]0;nvim\x07\x1b]7;file:///home/user\x07");
    assert_eq!(screen.title, "nvim");
    assert_eq!(screen.cwd, "file:///home/user");
    let (screen, _) = run(b"\x1b]2;editor\x1b\\");
    assert_eq!(screen.title, "editor");
  }

  #[test]
  fn osc52_writes_the_clipboard() {
    let (screen, _) = run(b"\x1b]52;c;aGVsbG8=\x07");
    assert_eq!(screen.clipboard_write.as_deref(), Some("hello"));
  }

  #[test]
  fn osc10_answers_with_the_foreground_color() {
    let (_, reply) = run(b"\x1b]10;?\x07");
    let text = String::from_utf8_lossy(&reply).to_string();
    assert!(text.starts_with("\x1b]10;rgb:"), "got {text:?}");
  }

  #[test]
  fn alt_screen_switches_grids() {
    let (screen, _) = run(b"main\x1b[?1049h");
    assert!(screen.modes.alternate());
    let (screen, _) = run(b"main\x1b[?1049htop\x1b[?1049l");
    assert!(!screen.modes.alternate());
    assert_eq!(typed(&screen), "main");
  }

  #[test]
  fn private_modes_toggle() {
    let (screen, _) = run(b"\x1b[?25l");
    assert!(!screen.modes.cursor_visible);
    let (screen, _) = run(b"\x1b[?25l\x1b[?2004h\x1b[?1002h\x1b[?1006h");
    assert!(screen.modes.bracketed_paste);
    assert!(screen.modes.mouse_drag);
    assert!(screen.modes.mouse_sgr);
    assert!(screen.modes.mouse_active());
    let (screen, _) = run(b"\x1b[?1h");
    assert!(screen.modes.app_cursor_keys);
  }

  #[test]
  fn insert_mode_and_edit_sequences() {
    let (screen, _) = run(b"abcd\x1b[1;3H\x1b[2P");
    assert_eq!(screen.line(0).unwrap().text(), "ab", "two cells deleted");
    let (screen, _) = run(b"abcd\x1b[1;3H\x1b[1P");
    assert_eq!(screen.line(0).unwrap().text(), "abd", "one cell deleted");
    let (screen, _) = run(b"abcd\x1b[1;3H\x1b[2@");
    assert_eq!(screen.line(0).unwrap().text(), "ab  cd");
    let (screen, _) = run(b"abcd\x1b[1;3H\x1b[4hX");
    assert_eq!(screen.line(0).unwrap().text(), "abXcd");
    let (screen, _) = run(b"abcd\x1b[1;2H\x1b[2X");
    assert_eq!(screen.line(0).unwrap().text(), "a  d");
  }

  #[test]
  fn scroll_region_homes_the_cursor() {
    let (screen, _) = run(b"\x1b[2;4r");
    assert_eq!(screen.cursor_y, 0);
    assert_eq!(screen.cursor_x, 0);
    assert_eq!(screen.cursor_line(), screen.history_len());
  }

  #[test]
  fn device_status_replies() {
    let (_, reply) = run(b"\x1b[6n");
    assert_eq!(reply, b"\x1b[1;1R");
    let (_, reply) = run(b"\x1b[5n");
    assert_eq!(reply, b"\x1b[0n");
    let (_, reply) = run(b"\x1b[c");
    assert_eq!(reply, b"\x1b[?1;2c");
  }

  #[test]
  fn special_graphics_charset() {
    let (screen, _) = run(b"\x1b(0lqk\x1b(B");
    assert_eq!(typed(&screen), "\u{250c}\u{2500}\u{2510}");
    let (screen, _) = run(b"\x1b)0\x0elqk\x0f");
    assert_eq!(typed(&screen), "\u{250c}\u{2500}\u{2510}");
  }

  #[test]
  fn save_and_restore_cursor() {
    let (screen, _) = run(b"\x1b[3;5H\x1b7\x1b[1;1H\x1b8");
    assert_eq!(screen.cursor_y, 2);
    assert_eq!(screen.cursor_x, 4);
  }

  #[test]
  fn cursor_shape_sequence() {
    let (screen, _) = run(b"\x1b[5 q");
    assert_eq!(screen.shape, CursorShape::Bar);
    let (screen, _) = run(b"\x1b[4 q");
    assert_eq!(screen.shape, CursorShape::Underline);
    let (screen, _) = run(b"\x1b[2 q");
    assert_eq!(screen.shape, CursorShape::Block);
  }

  #[test]
  fn utf8_across_chunks() {
    let mut screen = Screen::new(20, 5);
    let mut parser = Parser::new();
    let mut reply = Vec::new();
    parser.feed(b"a\xc3", &mut screen, &mut reply, 0.0);
    parser.feed(b"\xb6z", &mut screen, &mut reply, 0.0);
    assert_eq!(typed(&screen), "a\u{f6}z");
  }

  #[test]
  fn tab_stops_and_back_tab() {
    let (screen, _) = run(b"a\tb\tc");
    assert_eq!(screen.line(0).unwrap().text(), "a       b       c");
    let (screen, _) = run(b"a\tb\t\x1b[Z");
    assert_eq!(screen.cursor_x, 8);
  }

  #[test]
  fn erase_display_clears_scrollback() {
    let (screen, _) = run(b"a\r\nb\r\nc\r\nd\r\ne\x1b[3J");
    assert_eq!(screen.history_len(), 0);
  }

  #[test]
  fn repeat_last_character() {
    let (screen, _) = run(b"a\x1b[3b");
    assert_eq!(typed(&screen), "aaaa");
  }

  #[test]
  fn decaln_fills_the_grid() {
    let (screen, _) = run(b"\x1b[2J\x1b#8");
    assert_eq!(screen.line(0).unwrap().text(), "EEEEEEEEEEEEEEEEEEEE");
  }

  #[test]
  fn ris_resets_everything() {
    let (screen, _) = run(b"junk\x1b[?25l\x1bc");
    assert!(screen.modes.cursor_visible);
    assert_eq!(typed(&screen), "");
  }

  #[test]
  fn base64_decodes_osc52() {
    assert_eq!(decode_base64("aGVsbG8=").as_deref(), Some("hello"));
    assert_eq!(decode_base64("").as_deref(), Some(""));
  }

  #[test]
  fn bell_sets_the_flash_time() {
    let mut screen = Screen::new(20, 5);
    let mut parser = Parser::new();
    let mut reply = Vec::new();
    parser.feed(b"\x07", &mut screen, &mut reply, 3.5);
    assert_eq!(screen.bell_at, Some(3.5));
  }

  #[test]
  fn hyperlink_target_is_captured() {
    let (screen, _) = run(b"\x1b]8;;https://tontoo.os\x1b\\link");
    assert_eq!(screen.hyperlink.as_deref(), Some("https://tontoo.os"));
  }
}