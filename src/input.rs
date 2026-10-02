//! Input encoding: turns key presses, mouse reports and pasted text
//! into the byte sequences a shell or full screen program expects.
//!
//! The encoding follows xterm: application cursor keys and the keypad
//! switch to `SS3` sequences, function keys use their `CSI` numbers,
//! modifiers are appended as `1;mod` and mouse reports use SGR (`1006`)
//! when the program enabled it, X10 encoding otherwise.

use crate::TontooUI::renderer::window::{Modifiers, RawKey};

use crate::grid::Modes;

/// Key modifier bits: 1 shift, 2 alt, 4 ctrl, 8 meta.
fn modifier_bits(modifiers: Modifiers) -> u8 {
  let mut value = 0;
  if modifiers.shift {
    value |= 1;
  }
  if modifiers.alt {
    value |= 2;
  }
  if modifiers.ctrl {
    value |= 4;
  }
  if modifiers.super_key {
    value |= 8;
  }
  value
}

/// Modifier parameter of a key sequence (`CSI 1 ; mod A`): the xterm
/// form is the bit mask plus one.
fn modifier_param(modifiers: Modifiers) -> u8 {
  modifier_bits(modifiers) + 1
}

/// Modifier contribution of a mouse report: xterm weights shift 4, alt 8
/// and ctrl 16.
fn mouse_modifier_bits(modifiers: Modifiers) -> u8 {
  let mut value = 0;
  if modifiers.shift {
    value += 4;
  }
  if modifiers.alt {
    value += 8;
  }
  if modifiers.ctrl {
    value += 16;
  }
  value
}

/// One key press plus the decoded text, as the terminal sees it.
pub struct KeyInput {
  pub key: RawKey,
  pub modifiers: Modifiers,
  /// Decoded text for printable keys, including key repeat.
  pub text: Option<String>,
}

/// Bytes for one key press.
pub fn encode_key(input: &KeyInput, modes: &Modes) -> Vec<u8> {
  let alt = input.modifiers.alt;
  let ctrl = input.modifiers.ctrl;
  let mut out: Vec<u8> = Vec::with_capacity(16);
  let key = input.key;
  let text = input.text.clone().unwrap_or_default();

  match key {
    RawKey::Character(_) => {
      if ctrl {
        // Ctrl+letter becomes the matching control byte; the rest of the
        // keyboard has no control byte and is dropped.
        let ch = text.chars().next().unwrap_or('\0');
        let lowered = ch.to_ascii_lowercase();
        let code = match lowered {
          'a'..='z' => (lowered as u8) - b'a' + 1,
          '[' => 0x1b,
          '\\' => 0x1c,
          ']' => 0x1d,
          '^' => 0x1e,
          '_' => 0x1f,
          ' ' => 0x00,
          '?' => 0x7f,
          _ => 0,
        };
        if code == 0 {
          return out;
        }
        if alt {
          out.push(0x1b);
        }
        out.push(code);
        return out;
      }
      if alt {
        out.push(0x1b);
      }
      out.extend_from_slice(text.as_bytes());
      out
    }
    RawKey::Enter | RawKey::KeypadEnter => {
      if alt {
        out.push(0x1b);
      }
      out.push(b'\r');
      out
    }
    RawKey::Tab => {
      if alt {
        out.push(0x1b);
      }
      out.push(b'\t');
      out
    }
    RawKey::BackTab => b"\x1b[Z".to_vec(),
    RawKey::Escape => {
      out.push(0x1b);
      out
    }
    RawKey::Backspace => {
      if alt {
        out.push(0x1b);
      }
      // Terminals expect DEL, not BS.
      out.push(0x7f);
      out
    }
    RawKey::Delete => csi_number(3, input.modifiers),
    RawKey::Insert => csi_number(2, input.modifiers),
    RawKey::PageUp => csi_number(5, input.modifiers),
    RawKey::PageDown => csi_number(6, input.modifiers),
    RawKey::Left => arrow('D', modes.app_cursor_keys, input.modifiers),
    RawKey::Right => arrow('C', modes.app_cursor_keys, input.modifiers),
    RawKey::Up => arrow('A', modes.app_cursor_keys, input.modifiers),
    RawKey::Down => arrow('B', modes.app_cursor_keys, input.modifiers),
    RawKey::Home => home_end('H', modes.app_cursor_keys, input.modifiers),
    RawKey::End => home_end('F', modes.app_cursor_keys, input.modifiers),
    RawKey::Function(number) => function_key(number, input.modifiers),
    RawKey::KeypadDigit(digit) => keypad_digit(digit, modes.app_keypad, input.modifiers),
    RawKey::KeypadDot => keypad_special('n', modes.app_keypad, input.modifiers),
    RawKey::KeypadPlus => keypad_special('k', modes.app_keypad, input.modifiers),
    RawKey::KeypadMinus => keypad_special('m', modes.app_keypad, input.modifiers),
    RawKey::KeypadStar => keypad_special('j', modes.app_keypad, input.modifiers),
    RawKey::KeypadSlash => keypad_special('o', modes.app_keypad, input.modifiers),
    _ => Vec::new(),
  }
}

/// `CSI <number> ~` with the modifier parameter.
fn csi_number(number: u8, modifiers: Modifiers) -> Vec<u8> {
  let param = modifier_param(modifiers);
  if param == 1 {
    format!("\x1b[{number}~").into_bytes()
  } else {
    format!("\x1b[{number};{param}~").into_bytes()
  }
}

/// Arrow keys: `CSI A` normally, `SS3 A` in application mode.
fn arrow(final_byte: char, application: bool, modifiers: Modifiers) -> Vec<u8> {
  if application && !modifiers.any() {
    return vec![0x1b, b'O', final_byte as u8];
  }
  let param = modifier_param(modifiers);
  if param == 1 {
    format!("\x1b[{final_byte}").into_bytes()
  } else {
    format!("\x1b[1;{param}{final_byte}").into_bytes()
  }
}

/// Home and End follow the arrow rules.
fn home_end(final_byte: char, application: bool, modifiers: Modifiers) -> Vec<u8> {
  arrow(final_byte, application, modifiers)
}

/// F1 to F12: `SS3 P..S` for F1 to F4, `CSI n ~` for the rest.
fn function_key(number: u8, modifiers: Modifiers) -> Vec<u8> {
  match number {
    1 => arrow('P', true, modifiers),
    2 => arrow('Q', true, modifiers),
    3 => arrow('R', true, modifiers),
    4 => arrow('S', true, modifiers),
    5 => csi_number(15, modifiers),
    6 => csi_number(17, modifiers),
    7 => csi_number(18, modifiers),
    8 => csi_number(19, modifiers),
    9 => csi_number(20, modifiers),
    10 => csi_number(21, modifiers),
    11 => csi_number(23, modifiers),
    12 => csi_number(24, modifiers),
    _ => Vec::new(),
  }
}

/// Keypad digits: plain text unless the application keypad is active,
/// then `SS3 p..y`.
fn keypad_digit(digit: u8, application: bool, modifiers: Modifiers) -> Vec<u8> {
  if application && !modifiers.any() {
    let final_byte = match digit {
      0 => 'p',
      1 => 'q',
      2 => 'r',
      3 => 's',
      4 => 't',
      5 => 'u',
      6 => 'v',
      7 => 'w',
      8 => 'x',
      _ => 'y',
    };
    return vec![0x1b, b'O', final_byte as u8];
  }
  vec![b'0' + digit]
}

/// Keypad operators with their `SS3` finals.
fn keypad_special(final_byte: char, application: bool, modifiers: Modifiers) -> Vec<u8> {
  if application && !modifiers.any() {
    return vec![0x1b, b'O', final_byte as u8];
  }
  let symbol = match final_byte {
    'n' => '.',
    'k' => '+',
    'm' => '-',
    'j' => '*',
    _ => '/',
  };
  vec![symbol as u8]
}

/// What the pointer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerAction {
  Press,
  Release,
  Move,
}

/// Wheel direction of a scroll event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wheel {
  Up,
  Down,
}

/// Encode a mouse report. `button` is 0 left, 1 middle, 2 right, 3 none
/// (motion). `wheel` turns the event into a wheel report. Returns
/// `None` when no mouse mode is active or the mode does not cover this
/// action.
pub fn encode_mouse(
  modes: &Modes,
  action: PointerAction,
  button: u8,
  column: u16,
  row: u16,
  modifiers: Modifiers,
  wheel: Option<Wheel>,
) -> Option<Vec<u8>> {
  if !modes.mouse_active() {
    return None;
  }
  let mut code = match button {
    0 | 3 => 0,
    1 => 1,
    2 => 2,
    _ => 3,
  };
  if let Some(direction) = wheel {
    code = match direction {
      Wheel::Up => 64,
      Wheel::Down => 65,
    };
  } else if matches!(action, PointerAction::Move) {
    match modes.mouse_any {
      true => code += 32,
      false if modes.mouse_drag && button != 3 => code += 32,
      false => return None,
    }
  }
  code += mouse_modifier_bits(modifiers);
  let column = column.max(1);
  let row = row.max(1);
  if modes.mouse_sgr {
    let final_byte = match action {
      PointerAction::Release => 'm',
      _ => 'M',
    };
    return Some(format!("\x1b[<{code};{column};{row}{final_byte}").into_bytes());
  }
  if column > 223 || row > 223 {
    return None;
  }
  let mut out = vec![0x1b, b'[', b'M', 32 + code, 32 + column as u8, 32 + row as u8];
  if matches!(action, PointerAction::Release) && button != 3 {
    // X10 has no release: the button reports as released (3) instead.
    out[3] = 32 + 3;
  }
  Some(out)
}

/// Clipboard text as terminal input, wrapped in the bracketed paste
/// markers when the program asked for them.
pub fn encode_paste(text: &str, modes: &Modes) -> Vec<u8> {
  let mut out = Vec::with_capacity(text.len() + 16);
  if modes.bracketed_paste {
    out.extend_from_slice(b"\x1b[200~");
  }
  for ch in text.chars() {
    match ch {
      // A raw control byte would end the paste early and confuse the
      // program, so it is escaped the way a shell does it.
      '\n' => out.extend_from_slice(b"\r"),
      '\r' => out.extend_from_slice(b"\r"),
      other => {
        let mut buffer = [0u8; 4];
        out.extend_from_slice(other.encode_utf8(&mut buffer).as_bytes());
      }
    }
  }
  if modes.bracketed_paste {
    out.extend_from_slice(b"\x1b[201~");
  }
  out
}

/// Focus in / out reports for mode 1004.
pub fn encode_focus(focused: bool) -> Vec<u8> {
  if focused {
    b"\x1b[I".to_vec()
  } else {
    b"\x1b[O".to_vec()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn modes() -> Modes {
    Modes::default()
  }

  fn none() -> Modifiers {
    Modifiers::default()
  }

  fn key(key: RawKey) -> KeyInput {
    KeyInput {
      key,
      modifiers: none(),
      text: None,
    }
  }

  fn typed(key: RawKey, text: &str) -> KeyInput {
    KeyInput {
      key,
      modifiers: none(),
      text: Some(text.to_string()),
    }
  }

  #[test]
  fn plain_characters_are_sent_as_text() {
    let input = typed(RawKey::Character('a'), "a");
    assert_eq!(encode_key(&input, &modes()), b"a");
    // The platform already applies Shift, so the text is upper case.
    let input = typed(RawKey::Character('A'), "A");
    assert_eq!(encode_key(&input, &modes()), b"A");
  }

  #[test]
  fn control_letters_map_to_control_bytes() {
    for (ch, code) in [('c', 0x03u8), ('d', 0x04), ('z', 0x1a), ('l', 0x0c)] {
      let mut input = typed(RawKey::Character(ch), ch.to_string().as_str());
      input.modifiers.ctrl = true;
      assert_eq!(encode_key(&input, &modes()), vec![code], "ctrl+{ch}");
    }
  }

  #[test]
  fn alt_prefixes_with_escape() {
    let mut input = typed(RawKey::Character('b'), "b");
    input.modifiers.alt = true;
    assert_eq!(encode_key(&input, &modes()), vec![0x1b, b'b']);
    let mut input = key(RawKey::Enter);
    input.modifiers.alt = true;
    assert_eq!(encode_key(&input, &modes()), vec![0x1b, b'\r']);
  }

  #[test]
  fn arrows_follow_the_application_mode() {
    assert_eq!(
      encode_key(&key(RawKey::Up), &modes()),
      b"\x1b[A".to_vec()
    );
    let mut application = modes();
    application.app_cursor_keys = true;
    assert_eq!(encode_key(&key(RawKey::Up), &application), b"\x1bOA".to_vec());
    assert_eq!(
      encode_key(&key(RawKey::End), &application),
      b"\x1bOF".to_vec()
    );
  }

  #[test]
  fn modified_arrows_carry_the_parameter() {
    let mut input = key(RawKey::Left);
    input.modifiers.ctrl = true;
    assert_eq!(encode_key(&input, &modes()), b"\x1b[1;5D".to_vec());
    let mut input = key(RawKey::Right);
    input.modifiers.shift = true;
    assert_eq!(encode_key(&input, &modes()), b"\x1b[1;2C".to_vec());
  }

  #[test]
  fn navigation_and_function_keys() {
    assert_eq!(encode_key(&key(RawKey::Home), &modes()), b"\x1b[H".to_vec());
    assert_eq!(
      encode_key(&key(RawKey::Delete), &modes()),
      b"\x1b[3~".to_vec()
    );
    assert_eq!(
      encode_key(&key(RawKey::PageUp), &modes()),
      b"\x1b[5~".to_vec()
    );
    assert_eq!(encode_key(&key(RawKey::Function(1)), &modes()), b"\x1bOP".to_vec());
    assert_eq!(encode_key(&key(RawKey::Function(5)), &modes()), b"\x1b[15~".to_vec());
    assert_eq!(
      encode_key(&key(RawKey::Function(12)), &modes()),
      b"\x1b[24~".to_vec()
    );
  }

  #[test]
  fn tab_back_tab_and_backspace() {
    assert_eq!(encode_key(&key(RawKey::Tab), &modes()), b"\t".to_vec());
    assert_eq!(
      encode_key(&key(RawKey::BackTab), &modes()),
      b"\x1b[Z".to_vec()
    );
    assert_eq!(
      encode_key(&key(RawKey::Backspace), &modes()),
      vec![0x7f]
    );
  }

  #[test]
  fn keypad_switches_to_application_finals() {
    let mut application = modes();
    application.app_keypad = true;
    assert_eq!(
      encode_key(&key(RawKey::KeypadDigit(4)), &application),
      b"\x1bOt".to_vec()
    );
    assert_eq!(
      encode_key(&key(RawKey::KeypadEnter), &application),
      b"\r".to_vec()
    );
    assert_eq!(encode_key(&key(RawKey::KeypadDigit(4)), &modes()), b"4".to_vec());
  }

  #[test]
  fn mouse_needs_an_active_mode() {
    assert!(encode_mouse(&modes(), PointerAction::Press, 0, 4, 5, none(), None).is_none());
  }

  #[test]
  fn sgr_mouse_reports() {
    let mut active = modes();
    active.mouse_button = true;
    active.mouse_sgr = true;
    let press = encode_mouse(&active, PointerAction::Press, 0, 4, 5, none(), None).unwrap();
    assert_eq!(press, b"\x1b[<0;4;5M");
    let release = encode_mouse(&active, PointerAction::Release, 0, 4, 5, none(), None).unwrap();
    assert_eq!(release, b"\x1b[<0;4;5m");
    let mut shift = none();
    shift.shift = true;
    let report = encode_mouse(&active, PointerAction::Press, 0, 1, 1, shift, None).unwrap();
    assert_eq!(report, b"\x1b[<4;1;1M", "xterm weights shift as 4");
  }

  #[test]
  fn x10_mouse_reports() {
    let mut active = modes();
    active.mouse_button = true;
    let press = encode_mouse(&active, PointerAction::Press, 0, 1, 1, none(), None).unwrap();
    assert_eq!(press, b"\x1b[M\x20\x21\x21");
    let release = encode_mouse(&active, PointerAction::Release, 0, 1, 1, none(), None).unwrap();
    assert_eq!(release[3], 35, "released button reports as 3");
  }

  #[test]
  fn drag_and_wheel_reports() {
    let mut drag = modes();
    drag.mouse_drag = true;
    drag.mouse_sgr = true;
    let report = encode_mouse(&drag, PointerAction::Move, 0, 2, 2, none(), None).unwrap();
    assert_eq!(report, b"\x1b[<32;2;2M");
    let wheel = encode_mouse(&drag, PointerAction::Move, 0, 2, 2, none(), Some(Wheel::Up)).unwrap();
    assert_eq!(wheel, b"\x1b[<64;2;2M");
    let mut press_only = modes();
    press_only.mouse_button = true;
    press_only.mouse_sgr = true;
    assert!(encode_mouse(&press_only, PointerAction::Move, 0, 2, 2, none(), None).is_none());
  }

  #[test]
  fn bracketed_paste_wraps_the_text() {
    let mut bracketed = modes();
    bracketed.bracketed_paste = true;
    assert_eq!(
      encode_paste("ls\n", &bracketed),
      b"\x1b[200~ls\r\x1b[201~".to_vec()
    );
    assert_eq!(encode_paste("ls", &modes()), b"ls".to_vec());
  }

  #[test]
  fn focus_reports() {
    assert_eq!(encode_focus(true), b"\x1b[I".to_vec());
    assert_eq!(encode_focus(false), b"\x1b[O".to_vec());
  }
}