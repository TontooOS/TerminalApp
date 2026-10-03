//! Colors: TontooOS theme tokens, the ANSI palette, 256 color
//! resolution and the OSC 10 / 11 report.
//!
//! The window and the grid background follow the TontooOS tokens:
//! `#1b2022` with `#d8d9d9` text in dark mode, `#ffffff` with
//! `#272727` text in light mode. Program output keeps the classic
//! 16 color ANSI palette plus the xterm 256 color cube.

use crate::TontooUI::Color;

use crate::config;
use crate::grid::Color as CellColor;

/// Parse `#rrggbb` into a peniko color. Invalid input falls back to
/// opaque white so a typo can never produce an invisible surface.
pub fn hex_color(hex: &str) -> Color {
  let hex = hex.trim().trim_start_matches('#');
  if hex.len() != 6 {
    return Color::WHITE;
  }
  let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(255);
  Color::from_rgb8(channel(0), channel(2), channel(4))
}

/// Grid background in the active mode.
pub fn background(dark: bool) -> Color {
  hex_color(if dark {
    config::BG_DARK
  } else {
    config::BG_LIGHT
  })
}

/// Default grid text color in the active mode.
pub fn foreground(dark: bool) -> Color {
  hex_color(if dark {
    config::FG_DARK
  } else {
    config::FG_LIGHT
  })
}

/// Cursor fill: the grid text color, so the block hides the glyph under
/// it like the native terminal cursor does.
pub fn cursor_fill(dark: bool) -> Color {
  foreground(dark)
}

/// Title bar text of the mode, matching the `Titlebar` tokens.
pub fn titlebar_text(dark: bool) -> Color {
  if dark {
    Color::from_rgb8(0xd8, 0xd9, 0xd9)
  } else {
    Color::from_rgb8(0x27, 0x27, 0x27)
  }
}

/// Selection highlight: a clearly visible veil over the cell colors,
/// light enough to keep the glyphs readable.
pub fn selection_fill(dark: bool) -> Color {
  if dark {
    Color::from_rgba8(0x6e, 0x93, 0xad, 0xb3)
  } else {
    Color::from_rgba8(0x9c, 0xc4, 0xf0, 0xb3)
  }
}

/// Resolve a cell color against the active mode. `Default` becomes the
/// theme text color; palette entries use the ANSI table and the xterm
/// 256 color cube.
pub fn resolve(color: CellColor, dark: bool) -> Color {
  match color {
    CellColor::Default => foreground(dark),
    CellColor::Rgb(r, g, b) => Color::from_rgb8(r, g, b),
    CellColor::Indexed(index) => indexed(index, dark),
  }
}

/// Resolve a cell background. `Default` means "the grid background", so
/// it is resolved by the caller with [`background`].
pub fn resolve_background(color: CellColor, dark: bool) -> Option<Color> {
  match color {
    CellColor::Default => None,
    other => Some(resolve(other, dark)),
  }
}

/// xterm palette entry `0..=255`.
pub fn indexed(index: u8, dark: bool) -> Color {
  match index {
    0..=15 => {
      let hex = config::PALETTE[index as usize];
      match (index, dark) {
        // Black and bright black need lifting in dark mode, dimming in
        // light mode, otherwise they vanish into the background.
        (0, true) => hex_color("#1c1c1c"),
        (0, false) => hex_color("#1e1e1e"),
        (7, true) => hex_color("#c7c7c7"),
        (7, false) => hex_color("#3a3a3c"),
        (8, true) => hex_color("#8a8a8a"),
        (8, false) => hex_color("#6e6e73"),
        _ => hex_color(hex),
      }
    }
    16..=231 => {
      let value = index as u32 - 16;
      let steps = [0u8, 95, 135, 175, 215, 255];
      let r = steps[(value / 36) as usize];
      let g = steps[((value % 36) / 6) as usize];
      let b = steps[(value % 6) as usize];
      Color::from_rgb8(r, g, b)
    }
    _ => {
      let level = 8 + (index as u32 - 232) * 10;
      let level = level.min(238) as u8;
      Color::from_rgb8(level, level, level)
    }
  }
}

/// `rgb:rrrr/gggg/bbbb` report for OSC 10 and OSC 11, matching what
/// xterm answers to a color query. Built from the hex token so the
/// value is the configured one, never a converted round trip.
pub fn color_report(dark: bool, foreground_query: bool) -> String {
  let hex = if foreground_query {
    if dark {
      config::FG_DARK
    } else {
      config::FG_LIGHT
    }
  } else if dark {
    config::BG_DARK
  } else {
    config::BG_LIGHT
  };
  let hex = hex.trim().trim_start_matches('#');
  let channel = |at: usize| hex.get(at..at + 2).and_then(|part| u8::from_str_radix(part, 16).ok()).unwrap_or(0);
  let part = |value: u8| format!("{:02x}{:02x}", value, value);
  format!(
    "rgb:{}/{}/{}",
    part(channel(0)),
    part(channel(2)),
    part(channel(4))
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tokens_match_the_convention() {
    let dark_bg = background(true).to_rgba8();
    assert_eq!((dark_bg.r, dark_bg.g, dark_bg.b), (0x1b, 0x20, 0x22));
    let dark_fg = foreground(true).to_rgba8();
    assert_eq!((dark_fg.r, dark_fg.g, dark_fg.b), (0xd8, 0xd9, 0xd9));
    let light_bg = background(false).to_rgba8();
    assert_eq!((light_bg.r, light_bg.g, light_bg.b), (0xff, 0xff, 0xff));
  }

  #[test]
  fn light_text_is_the_light_token() {
    let color = foreground(false);
    assert_eq!(color.to_rgba8().r, 0x27);
    assert_eq!(color.to_rgba8().g, 0x27);
    assert_eq!(color.to_rgba8().b, 0x27);
  }

  #[test]
  fn hex_parser_handles_bad_input() {
    assert_eq!(hex_color("#ffffff").to_rgba8().r, 255);
    assert_eq!(hex_color("nope"), Color::WHITE);
  }

  #[test]
  fn color_cube_matches_xterm() {
    let cube = indexed(196, true).to_rgba8();
    assert_eq!((cube.r, cube.g, cube.b), (255, 0, 0));
    let gray = indexed(232, true).to_rgba8();
    assert_eq!(gray.r, gray.g);
    assert_eq!(gray.g, gray.b);
  }

  #[test]
  fn default_cell_colors_resolve_to_tokens() {
    assert_eq!(resolve(CellColor::Default, true), foreground(true));
    assert_eq!(resolve_background(CellColor::Default, true), None);
    assert!(resolve_background(CellColor::Indexed(1), true).is_some());
  }

  #[test]
  fn report_uses_four_hex_digits() {
    let report = color_report(true, true);
    assert_eq!(report, "rgb:d8d8/d9d9/d9d9");
    assert_eq!(color_report(false, false), "rgb:ffff/ffff/ffff");
  }
}