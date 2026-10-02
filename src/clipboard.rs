//! Clipboard access for copy and paste, backed by the TontooOS
//! Foundation framework with an in-process fallback.
//!
//! `NSPasteboard` talks to the display server natively; where that
//! fails (headless tests, missing server) the last copied text stays
//! readable inside the app so shortcuts keep working.

use std::sync::{Mutex, OnceLock};

/// System clipboard write. Returns true when the system clipboard took
/// the text; the fallback always mirrors it.
pub fn set(text: &str) -> bool {
  if let Ok(mut guard) = fallback().lock() {
    *guard = text.to_string();
  }
  crate::Foundation::pasteboard::Pasteboard::general()
    .set_text(text)
    .is_ok()
}

/// System clipboard read, falling back to the mirrored text.
pub fn get() -> Option<String> {
  if let Ok(Some(text)) = crate::Foundation::pasteboard::Pasteboard::general().get_text() {
    if !text.is_empty() {
      return Some(text);
    }
  }
  fallback()
    .lock()
    .ok()
    .and_then(|guard| {
      if guard.is_empty() {
        None
      } else {
        Some(guard.clone())
      }
    })
}

fn fallback() -> &'static Mutex<String> {
  static FALLBACK: OnceLock<Mutex<String>> = OnceLock::new();
  FALLBACK.get_or_init(|| Mutex::new(String::new()))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn roundtrip_works_without_a_display_server() {
    set("terminal clipboard");
    assert_eq!(get().as_deref(), Some("terminal clipboard"));
    set("");
    assert_eq!(get(), None);
  }
}