//! Terminal configuration: shell, font, colors, bell and scrollback.
//!
//! The default shell is `zsh`. Override with `TONTOO_TERMINAL_SHELL` or
//! `SHELL`. When the configured shell binary is missing, fall back to
//! `/bin/bash` and finally `/bin/sh` so the window always opens.

/// Default shell for TontooOS Terminal.
pub const DEFAULT_SHELL: &str = "/bin/zsh";

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

/// Terminal font. SF Mono is the TontooOS monospace face (SF family).
/// 9pt fits roughly twice as many cells as 13pt.
pub fn font_description() -> String {
  "SF Mono 9, Adwaita Mono 9, Monospace 9".to_string()
}

/// Scrollback lines kept in memory.
pub const SCROLLBACK_LINES: i64 = 10_000;

/// Audible bell (system beep) enabled by default.
pub const AUDIBLE_BELL: bool = true;

/// Visual bell flash enabled by default.
pub const VISUAL_BELL: bool = true;

/// TontooOS background tokens (match AGENTS.md).
pub const BG_DARK: &str = "#1d1d1d";
pub const BG_LIGHT: &str = "#ececec";
/// Foreground tokens. The prompt file paints `[user@machine folder]`
/// green; typed text and output stay in the normal foreground.
pub const FG_DARK: &str = "#F5F5F7";
pub const FG_LIGHT: &str = "#1E1E1E";

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
  fn dark_foreground_is_plain_white() {
    assert_eq!(FG_DARK, "#F5F5F7");
  }
}
