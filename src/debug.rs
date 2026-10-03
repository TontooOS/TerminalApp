//! Debug tracing, enabled with `TONTOO_TERMINAL_DEBUG=1`.
//!
//! Every trace goes to stderr, so the app itself stays quiet. The lines
//! are the input path end to end: what the shell reported, what the app
//! made of it and what went to the PTY.

use std::sync::OnceLock;

fn flag() -> bool {
  static ENABLED: OnceLock<bool> = OnceLock::new();
  *ENABLED.get_or_init(|| std::env::var("TONTOO_TERMINAL_DEBUG").is_ok())
}

/// True when tracing is on.
pub fn enabled() -> bool {
  flag()
}

/// Print one trace line to stderr when tracing is on.
pub fn trace(args: std::fmt::Arguments<'_>) {
  if flag() {
    eprintln!("terminal: {args}");
  }
}

/// Printable form of a byte slice for the trace: escapes control bytes
/// and non-ASCII, so a pasted or echoed sequence stays readable.
pub fn bytes(raw: &[u8]) -> String {
  let mut out = String::with_capacity(raw.len());
  for byte in raw {
    match byte {
      0x1b => out.push_str("<ESC>"),
      0x07 => out.push_str("<BEL>"),
      0x08 => out.push_str("<BS>"),
      0x09 => out.push_str("<TAB>"),
      0x0d => out.push_str("<CR>"),
      0x0a => out.push_str("<LF>"),
      0x00 => out.push_str("<NUL>"),
      0x20..=0x7e => out.push(*byte as char),
      other => out.push_str(&format!("<{other:02x}>")),
    }
  }
  out
}
