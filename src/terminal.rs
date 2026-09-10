//! VTE terminal view for TontooOS Terminal.
//!
//! A single `vte4::Terminal` fills the UIKit content area (macOS style:
//! decoration bar on top, terminal below, no extra input row needed because
//! VTE handles keyboard input, selection, colors and programs natively).
//! The widget instance is kept across UIKit rebuilds so the shell session
//! survives resizes, fullscreen toggles and theme changes.

use crate::config;
use gtk::prelude::*;
use std::cell::RefCell;

thread_local! {
  static TERMINAL: RefCell<Option<vte4::Terminal>> = RefCell::new(None);
  static OSC_TITLE: RefCell<String> = RefCell::new(String::new());
  static CWD_URI: RefCell<String> = RefCell::new(String::new());
}

fn rgba(hex: &str) -> gdk4::RGBA {
  gdk4::RGBA::parse(hex).unwrap_or_else(|_| gdk4::RGBA::BLACK)
}

/// Active palette: standard 16 ANSI colors.
fn palette() -> Vec<gdk4::RGBA> {
  config::PALETTE.iter().map(|c| rgba(c)).collect()
}

fn is_dark() -> bool {
  match crate::UIKit::app::current_color_scheme() {
    Some(crate::UIKit::prelude::ColorScheme::Light) => false,
    Some(crate::UIKit::prelude::ColorScheme::Dark) => true,
    None => {
      crate::UIKit::prelude::ColorScheme::detect_system()
        == crate::UIKit::prelude::ColorScheme::Dark
    }
  }
}

fn apply_theme(term: &vte4::Terminal) {
  use vte4::TerminalExtManual;
  let dark = is_dark();
  let fg = rgba(if dark { config::FG_DARK } else { config::FG_LIGHT });
  let bg = rgba(if dark { config::BG_DARK } else { config::BG_LIGHT });
  let pal = palette();
  let refs: Vec<&gdk4::RGBA> = pal.iter().collect();
  term.set_colors(Some(&fg), Some(&bg), &refs);
}

/// Walk the toplevel window and update the UIKit decoration-bar label.
fn push_title_to_chrome(term: &vte4::Terminal, title: &str) {
  let Some(root) = term.root() else { return };
  let Ok(window) = root.downcast::<gtk::Window>() else {
    return;
  };
  window.set_title(Some(title));
  if let Some(child) = window.child() {
    set_decoration_label(&child, title);
  }
}

fn set_decoration_label(widget: &gtk::Widget, title: &str) -> bool {
  if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
    if label.has_css_class("uikit-titlebar-title") {
      label.set_text(title);
      make_title_open_finder(&label);
      return true;
    }
  }
  let mut child = widget.first_child();
  while let Some(current) = child {
    let next = current.next_sibling();
    if set_decoration_label(&current, title) {
      return true;
    }
    child = next;
  }
  false
}

/// Make the decoration-bar title open the current folder in Finder.
/// Called on every title refresh; the click controller is attached only
/// once per label (rebuilds create fresh labels).
fn make_title_open_finder(label: &gtk::Label) {
  label.set_cursor_from_name(Some("pointer"));
  label.set_tooltip_text(Some(&crate::lang::t("title.open_in_finder")));
  let already = label
    .observe_controllers()
    .into_iter()
    .any(|item| item.map(|obj| obj.is::<gtk::GestureClick>()).unwrap_or(false));
  if already {
    return;
  }
  let click = gtk::GestureClick::new();
  click.connect_pressed(|_, _, _, _| open_cwd_in_finder());
  label.add_controller(click);
}

/// Current shell folder as a filesystem path (passwd-resolved home fallback).
fn cwd_path() -> String {
  let uri = CWD_URI.with(|c| c.borrow().clone());
  if let Some(path) = uri_to_path(&uri) {
    if !path.is_empty() {
      return path;
    }
  }
  config::home_dir()
}

/// Open the current folder in the file manager (native Finder first).
fn open_cwd_in_finder() {
  let dir = cwd_path();
  if std::process::Command::new("finder").arg(&dir).spawn().is_ok() {
    return;
  }
  let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
}

fn uri_to_path(uri: &str) -> Option<String> {
  if let Some(stripped) = uri.strip_prefix("file://") {
    // Strip host part (e.g. file://host/path -> /path).
    let path = stripped.find('/').map(|i| &stripped[i..]).unwrap_or(stripped);
    return Some(percent_decode(path));
  }
  None
}

fn percent_decode(input: &str) -> String {
  let mut out = String::with_capacity(input.len());
  let bytes = input.as_bytes();
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'%' && i + 2 < bytes.len() {
      if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
        out.push((h * 16 + l) as char);
        i += 3;
        continue;
      }
    }
    out.push(bytes[i] as char);
    i += 1;
  }
  out
}

fn hex_val(b: u8) -> Option<u8> {
  match b {
    b'0'..=b'9' => Some(b - b'0'),
    b'a'..=b'f' => Some(b - b'a' + 10),
    b'A'..=b'F' => Some(b - b'A' + 10),
    _ => None,
  }
}

fn basename(path: &str) -> &str {
  let trimmed = path.trim_end_matches('/');
  if trimmed.is_empty() {
    return "/";
  }
  trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// Display title: OSC program title wins, then current path, then fallback.
fn display_title() -> String {
  let osc = OSC_TITLE.with(|c| c.borrow().clone());
  if !osc.trim().is_empty() {
    return osc;
  }
  let uri = CWD_URI.with(|c| c.borrow().clone());
  if !uri.is_empty() {
    if let Some(path) = uri_to_path(&uri) {
      if !path.is_empty() {
        return path;
      }
    }
  }
  crate::lang::t("app.title")
}

fn refresh_title(term: &vte4::Terminal) {
  push_title_to_chrome(term, &display_title());
}

fn flash_visual_bell(term: &vte4::Terminal) {
  if !config::VISUAL_BELL {
    return;
  }
  term.set_opacity(0.75);
  let weak = term.downgrade();
  glib::timeout_add_local_once(std::time::Duration::from_millis(90), move || {
    if let Some(term) = weak.upgrade() {
      term.set_opacity(1.0);
    }
  });
}

fn spawn_shell(term: &vte4::Terminal) {
  use vte4::prelude::TerminalExtManual;
  let shell = config::resolve_shell();
  let home = config::home_dir();
  // Green [user@machine folder] prompt (zsh only): the generated $ZDOTDIR
  // sources the real user config first and our prompt last.
  let zdotdir = if config::shell_basename(&shell) == "zsh" {
    crate::prompt::prompt_path().and_then(|path| crate::prompt::prepare_zdotdir(&path))
  } else {
    None
  };
  // Inherit the current environment so PATH, LANG, TERM etc. survive.
  let mut env: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
  env.push(format!("TONTOO_REALHOME={home}"));
  if let Some(dir) = &zdotdir {
    env.push(format!("ZDOTDIR={}", dir.display()));
  }
  let env_refs: Vec<&str> = env.iter().map(|s| s.as_str()).collect();
  let argv = [shell.as_str()];
  let shell_name = shell.clone();
  term.spawn_async(
    vte4::PtyFlags::DEFAULT,
    Some(home.as_str()),
    &argv,
    &env_refs,
    glib::SpawnFlags::DEFAULT,
    || {},
    -1,
    None::<&gtk::gio::Cancellable>,
    move |result| {
      if let Err(err) = result {
        eprintln!("Terminal: failed to spawn {shell_name}: {err}");
      }
    },
  );
}

fn configure(term: &vte4::Terminal) {
  use vte4::prelude::TerminalExt;
  let font = pango::FontDescription::from_string(&config::font_description());
  term.set_font(Some(&font));
  term.set_scrollback_lines(config::SCROLLBACK_LINES as libc::c_long);
  term.set_scroll_on_output(true);
  term.set_scroll_on_keystroke(true);
  term.set_audible_bell(config::AUDIBLE_BELL);
  term.set_allow_hyperlink(true);
  term.set_cursor_blink_mode(vte4::CursorBlinkMode::System);
  term.set_cursor_shape(vte4::CursorShape::Block);
  term.set_mouse_autohide(true);
  term.set_bold_is_bright(true);
  term.set_size(80, 24);
  // Transparent VTE background: the UIKit window background
  // (Dark #1d1d1d / Light #ececec at 0.85 + blur) shows through.
  term.set_clear_background(false);
  crate::UIKit::widget::apply_css(
    &term.clone().upcast::<gtk::Widget>(),
    "vte-terminal { background-color: transparent; }",
  );
  apply_theme(term);
}

fn connect_signals(term: &vte4::Terminal) {
  use vte4::prelude::TerminalExt;
  let t = term.clone();
  term.connect_window_title_changed(move |term| {
    let title = term.window_title().map(|s| s.to_string()).unwrap_or_default();
    OSC_TITLE.with(|c| *c.borrow_mut() = title);
    refresh_title(term);
  });
  let _ = t;
  term.connect_current_directory_uri_changed(|term| {
    let uri = term
      .current_directory_uri()
      .map(|s| s.to_string())
      .unwrap_or_default();
    CWD_URI.with(|c| *c.borrow_mut() = uri);
    // Only overwrite the bar when no program set its own title.
    let has_osc = OSC_TITLE.with(|c| !c.borrow().trim().is_empty());
    if !has_osc {
      refresh_title(term);
    }
  });
  term.connect_child_exited(|term, _status| {
    // Minimal v1: respawn a fresh shell so the window stays usable.
    OSC_TITLE.with(|c| c.borrow_mut().clear());
    spawn_shell(term);
    refresh_title(term);
  });
  term.connect_bell(|term| {
    flash_visual_bell(term);
  });
}

/// Return the shared terminal, creating and spawning it on first use.
fn shared_terminal() -> vte4::Terminal {
  TERMINAL.with(|cell| {
    if let Some(term) = cell.borrow().as_ref() {
      apply_theme(term);
      return term.clone();
    }
    let term = vte4::Terminal::new();
    configure(&term);
    connect_signals(&term);
    spawn_shell(&term);
    // Seed the decoration bar immediately (before the first OSC title).
    let shell = config::resolve_shell();
    let _ = basename(&shell);
    *cell.borrow_mut() = Some(term.clone());
    term
  })
}

/// UIKit root widget: the shared VTE terminal in a full-size container.
pub struct TerminalRoot {
  id: crate::UIKit::widget::WidgetId,
}

impl TerminalRoot {
  pub fn new() -> Self {
    Self {
      id: crate::UIKit::widget::next_widget_id(),
    }
  }
}

impl crate::UIKit::prelude::Widget for TerminalRoot {
  fn id(&self) -> crate::UIKit::widget::WidgetId {
    self.id
  }

  fn to_gtk(&self) -> gtk::Widget {
    let term = shared_terminal();
    term.set_hexpand(true);
    term.set_vexpand(true);
    // Refresh the bar title on every rebuild (theme toggles rebuild too).
    refresh_title(&term);
    term.grab_focus();
    term.upcast()
  }

  fn is_interactive(&self) -> bool {
    true
  }

  fn fill_width(&self) -> bool {
    true
  }

  fn padding(&self) -> crate::UIKit::prelude::Padding {
    crate::UIKit::prelude::Padding::ZERO
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn decoration_title_prefers_osc_over_cwd() {
    OSC_TITLE.with(|c| *c.borrow_mut() = "nvim".to_string());
    CWD_URI.with(|c| *c.borrow_mut() = "file:///home/user/project".to_string());
    assert_eq!(display_title(), "nvim");
    OSC_TITLE.with(|c| c.borrow_mut().clear());
    assert_eq!(display_title(), "/home/user/project");
    CWD_URI.with(|c| c.borrow_mut().clear());
  }

  #[test]
  fn uri_to_path_strips_file_scheme() {
    assert_eq!(
      uri_to_path("file:///home/user/project").as_deref(),
      Some("/home/user/project")
    );
    assert_eq!(basename("/home/user/project/"), "project");
  }

  #[test]
  fn cwd_path_falls_back_to_existing_home() {
    CWD_URI.with(|c| c.borrow_mut().clear());
    let path = cwd_path();
    assert!(!path.is_empty());
    assert!(std::path::Path::new(&path).is_dir());
  }
}
