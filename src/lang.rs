//! Locale store for Terminal, backed by the TontooOS Accessibility
//! framework.
//!
//! Loads `lang/en_us.json` and `lang/de_de.json` from the app bundle
//! (or the checkout when running from source), picks the language from
//! the system locale (`LANGUAGE`, `LC_ALL`, `LANG`,
//! `/etc/locale.conf`) and looks strings up through the global
//! `LangStore`. A missing key returns the key itself, so a forgotten
//! translation shows up as text instead of an empty label.

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::Accessibility::{LangFile, LangStore};

static LOCALE: OnceLock<String> = OnceLock::new();

/// Detect the system locale. Returns `de_de` for German, `en_us`
/// otherwise.
pub fn detect_locale() -> String {
  for key in ["LANGUAGE", "LC_ALL", "LANG"] {
    if let Ok(value) = std::env::var(key) {
      let lower = value.to_lowercase();
      if lower.starts_with("de") {
        return "de_de".to_string();
      }
      if lower.starts_with("en") {
        return "en_us".to_string();
      }
    }
  }
  if let Ok(content) = std::fs::read_to_string("/etc/locale.conf") {
    if content.to_lowercase().contains("lang=de") {
      return "de_de".to_string();
    }
  }
  "en_us".to_string()
}

/// Directories that may hold the `lang/` folder: the bundle
/// (`Terminal.app/Resources/lang`), the checkout and the system share.
fn lang_dirs() -> Vec<PathBuf> {
  let mut dirs = Vec::new();
  if let Ok(cwd) = std::env::current_dir() {
    dirs.push(cwd.join("lang"));
  }
  if let Ok(exe) = std::env::current_exe() {
    if let Some(parent) = exe.parent() {
      dirs.push(parent.join("lang"));
      if let Some(grand) = parent.parent() {
        dirs.push(grand.join("lang"));
        dirs.push(grand.join("Resources").join("lang"));
      }
    }
  }
  dirs.push(PathBuf::from("/usr/share/terminal/lang"));
  dirs
}

/// Load both languages into the global store. Safe to call repeatedly.
pub fn init() {
  let locale = detect_locale();
  let mut files = Vec::new();
  for code in ["en_us", "de_de"] {
    let name = format!("{code}.json");
    for dir in lang_dirs() {
      let path = dir.join(&name);
      if let Ok(file) = LangFile::from_file(&path) {
        files.push(file);
        break;
      }
    }
  }
  // The fallback must be one of the loaded files, so an incomplete
  // checkout still initialises with English.
  if !files.iter().any(|file| file.lang == "en_us") {
    return;
  }
  let _ = LangStore::init(files, Some("en_us".to_string()));
  let _ = LOCALE.set(locale);
}

/// Look up a localized string. Returns the key itself when missing.
pub fn t(key: &str) -> String {
  init();
  let locale = LOCALE.get().map(String::as_str).unwrap_or("en_us");
  LangStore::instance()
    .t(locale, key, None)
    .unwrap_or_else(|| key.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn detect_defaults_to_a_supported_locale() {
    let locale = detect_locale();
    assert!(locale == "en_us" || locale == "de_de");
  }

  #[test]
  fn missing_key_returns_the_key() {
    assert_eq!(t("missing.key.that.does.not.exist"), "missing.key.that.does.not.exist");
  }

  #[test]
  fn known_keys_translate() {
    init();
    assert_eq!(t("app.title"), "Terminal");
    assert!(!t("title.open_in_finder").is_empty());
  }
}