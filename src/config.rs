//! Terminal configuration: shell, font, colors, bell and scrollback.
//!
//! The default shell is `zsh`. Override with `TONTOO_TERMINAL_SHELL` or
//! `SHELL`. When the configured shell binary is missing, fall back to
//! `/bin/bash` and finally `/bin/sh` so the window always opens.
//!
//! Colors follow the TontooOS tokens: background `#1b2022` with text
//! `#d8d9d9` in dark mode, background `#ffffff` with text `#272727` in
//! light mode. Program output keeps its own ANSI palette.

/// Window size in logical px. The content viewport sits inside the
/// window frame, so the terminal area is a little smaller: the frame
/// takes 24 px on each side, which leaves a 756x511 viewport and a
/// 96x27 grid at 13 px.
pub const WINDOW_WIDTH: u32 = 804;
pub const WINDOW_HEIGHT: u32 = 559;

/// Default shell for TontooOS Terminal.
pub const DEFAULT_SHELL: &str = "/bin/zsh";

/// Environment variable overriding the font size in logical px.
pub const FONT_SIZE_ENV: &str = "TONTOO_TERMINAL_FONT_SIZE";

/// Terminal font size in logical px. The cell width is measured from
/// the resolved font, so the grid always aligns.
pub const FONT_SIZE: f32 = 13.0;

/// Smallest font size accepted from `TONTOO_TERMINAL_FONT_SIZE`.
const FONT_SIZE_MIN: f32 = 6.0;

/// Largest font size accepted from `TONTOO_TERMINAL_FONT_SIZE`.
const FONT_SIZE_MAX: f32 = 72.0;

/// Resolve the shell binary to spawn.
pub fn resolve_shell() -> String {
  for key in ["TONTOO_TERMINAL_SHELL", "SHELL"] {
    if let Ok(value) = std::env::var(key) {
      let value = value.trim().to_string();
      if !value.is_empty() && std::path::Path::new(&value).exists() {
        return value;
      }
    }
  }
  if std::path::Path::new(DEFAULT_SHELL).exists() {
    return DEFAULT_SHELL.to_string();
  }
  for fallback in ["/bin/bash", "/bin/sh"] {
    if std::path::Path::new(fallback).exists() {
      return fallback.to_string();
    }
  }
  DEFAULT_SHELL.to_string()
}

/// Short shell name for fallback titles (e.g. `/bin/zsh` -> `zsh`).
pub fn shell_basename(shell: &str) -> &str {
  shell.rsplit('/').next().unwrap_or(shell)
}

/// Home directory of the current user.
///
/// `$HOME` wins when it points to an existing directory (covers `su`
/// without login where `HOME` still points at the real home). Otherwise
/// the passwd entry for the current uid is used, so root reliably lands
/// in `/root` instead of `/`. Last resort is `/tmp`, never `/`.
pub fn home_dir() -> String {
  if let Ok(value) = std::env::var("HOME") {
    if !value.is_empty() && std::path::Path::new(&value).is_dir() {
      return value;
    }
  }
  if let Some(dir) = passwd_home() {
    if std::path::Path::new(&dir).is_dir() {
      return dir;
    }
  }
  "/tmp".to_string()
}

/// Home directory from the passwd database for the current uid.
fn passwd_home() -> Option<String> {
  unsafe {
    let pwd = libc::getpwuid(libc::getuid());
    if pwd.is_null() || (*pwd).pw_dir.is_null() {
      return None;
    }
    std::ffi::CStr::from_ptr((*pwd).pw_dir)
      .to_str()
      .ok()
      .map(|s| s.to_string())
  }
}

/// Font size for the grid, overridable with `TONTOO_TERMINAL_FONT_SIZE`.
/// Values outside `FONT_SIZE_MIN..=FONT_SIZE_MAX` fall back to
/// `FONT_SIZE`.
pub fn font_size() -> f32 {
  match std::env::var(FONT_SIZE_ENV) {
    Ok(raw) => match raw.trim().parse::<f32>() {
      Ok(size) if size >= FONT_SIZE_MIN && size <= FONT_SIZE_MAX => size,
      _ => FONT_SIZE,
    },
    Err(_) => FONT_SIZE,
  }
}

/// Scrollback lines kept in memory. The alternate screen never
/// scrollbacks.
pub const SCROLLBACK_LINES: usize = 10_000;

/// Visual bell flash enabled by default.
pub const VISUAL_BELL: bool = true;

/// Visual bell flash duration in seconds.
pub const VISUAL_BELL_SECONDS: f64 = 0.12;

/// Blink period for text that asked for it with SGR 5 or 6. The cursor
/// itself is static, so the grid never flickers on its own.
pub const BLINK_SECONDS: f64 = 0.53;

/// TontooOS background token, dark mode.
pub const BG_DARK: &str = "#1b2022";
/// TontooOS text token, dark mode.
pub const FG_DARK: &str = "#d8d9d9";
/// TontooOS background token, light mode.
pub const BG_LIGHT: &str = "#ffffff";
/// TontooOS text token, light mode.
pub const FG_LIGHT: &str = "#272727";

/// Classic 16-color ANSI palette (macOS-like, readable on both schemes).
pub const PALETTE: [&str; 16] = [
  "#000000", "#c91b00", "#00c200", "#c7c400", "#0225c7", "#c930c7", "#00c5c7", "#c7c7c7",
  "#686868", "#ff6e67", "#5ffa68", "#fffc67", "#6871ff", "#ff77ff", "#60fdff", "#ffffff",
];

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn shell_basename_strips_path() {
    assert_eq!(shell_basename("/bin/zsh"), "zsh");
    assert_eq!(shell_basename("zsh"), "zsh");
  }

  #[test]
  fn resolve_shell_never_empty() {
    assert!(!resolve_shell().is_empty());
  }

  #[test]
  fn palette_has_sixteen_entries() {
    assert_eq!(PALETTE.len(), 16);
  }

  #[test]
  fn home_dir_is_existing_directory() {
    let home = home_dir();
    assert!(!home.is_empty());
    assert_ne!(home, "/");
    assert!(std::path::Path::new(&home).is_dir());
  }

  #[test]
  fn font_size_uses_default_without_env() {
    assert_eq!(font_size(), FONT_SIZE);
  }

  #[test]
  fn colors_are_the_tontooos_tokens() {
    assert_eq!(BG_DARK, "#1b2022");
    assert_eq!(FG_DARK, "#d8d9d9");
    assert_eq!(BG_LIGHT, "#ffffff");
    assert_eq!(FG_LIGHT, "#272727");
  }
}