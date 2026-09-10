//! Minimal locale store for Terminal.
//!
//! Loads `lang/en_us.json` or `lang/de_de.json` based on the system locale
//! (`LANGUAGE`, `LC_ALL`, `LANG`, `/etc/locale.conf`). Falls back to English
//! when no file matches. Only `en_us` and `de_de` are supported.

use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::path::PathBuf;

static STRINGS: OnceCell<HashMap<String, String>> = OnceCell::new();
static LOCALE: OnceCell<String> = OnceCell::new();

/// Detect the system locale. Returns `de_de` for German, `en_us` otherwise.
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

/// Candidate directories holding the `lang/` folder.
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
        // .app bundle layout: <Name>.app/{App/binary, Resources/lang}.
        dirs.push(grand.join("Resources").join("lang"));
      }
    }
  }
  dirs.push(PathBuf::from("/usr/share/terminal/lang"));
  dirs
}

fn load_map(locale: &str) -> HashMap<String, String> {
  let file = format!("{locale}.json");
  for dir in lang_dirs() {
    let path = dir.join(&file);
    if let Ok(content) = std::fs::read_to_string(&path) {
      if let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&content) {
        return map;
      }
    }
  }
  HashMap::new()
}

/// Load strings for the detected locale. Safe to call multiple times.
pub fn init() {
  if STRINGS.get().is_some() {
    return;
  }
  let locale = detect_locale();
  let map = load_map(&locale);
  let _ = LOCALE.set(locale);
  let _ = STRINGS.set(map);
}

/// Look up a localized string. Returns the key itself when missing.
pub fn t(key: &str) -> String {
  init();
  STRINGS
    .get()
    .and_then(|map| map.get(key))
    .cloned()
    .unwrap_or_else(|| key.to_string())
}

/// Active locale code (`en_us` or `de_de`).
#[allow(dead_code)]
pub fn locale() -> String {
  init();
  LOCALE.get().cloned().unwrap_or_else(|| "en_us".to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn detect_defaults_to_supported_locale() {
    let locale = detect_locale();
    assert!(locale == "en_us" || locale == "de_de");
  }

  #[test]
  fn missing_key_returns_key() {
    let value = t("missing.key.that.does.not.exist");
    assert_eq!(value, "missing.key.that.does.not.exist");
  }
}
