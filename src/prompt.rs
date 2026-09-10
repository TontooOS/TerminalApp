//! Green `[user@machine folder]` prompt that respects user config.
//!
//! zsh reads its rc files from `$ZDOTDIR`. We point it at a generated dir
//! whose `.zshenv` / `.zshrc` first delegate to the real files (`/etc/zsh`
//! plus the user home) and then source our prompt last, so the green prompt
//! wins while aliases, completions and themes keep working. Only used for
//! zsh; other shells are spawned untouched.

use std::path::{Path, PathBuf};

pub const PROMPT_FILE: &str = "tontoo-prompt.zsh";

/// Directories searched for the prompt file (dev checkout, bundle, system).
fn prompt_dirs() -> Vec<PathBuf> {
  let mut dirs = Vec::new();
  if let Ok(exe) = std::env::current_exe() {
    if let Some(dir) = exe.parent() {
      dirs.push(dir.join("Resources"));
      if let Some(grand) = dir.parent() {
        dirs.push(grand.join("Resources"));
      }
    }
  }
  if let Ok(cwd) = std::env::current_dir() {
    dirs.push(cwd.join("Resources"));
  }
  dirs.push(PathBuf::from("/usr/share/terminal"));
  dirs
}

/// Absolute path of the prompt file, if found.
pub fn prompt_path() -> Option<PathBuf> {
  prompt_dirs()
    .iter()
    .map(|dir| dir.join(PROMPT_FILE))
    .find(|path| path.is_file())
}

/// Runtime dir for the generated `$ZDOTDIR`.
fn zdotdir_base() -> PathBuf {
  if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
    if !runtime.is_empty() {
      return PathBuf::from(runtime).join("tontoo-terminal");
    }
  }
  let uid = unsafe { libc::getuid() };
  PathBuf::from(format!("/tmp/tontoo-terminal-{uid}"))
}

fn sh_quote(path: &Path) -> String {
  let escaped = path.to_string_lossy().replace('\'', "'\\''");
  format!("'{escaped}'")
}

/// Write delegating `.zshenv` / `.zshrc` into the runtime dir and return it.
/// Returns `None` when the dir is not writable (shell falls back to plain zsh).
pub fn prepare_zdotdir(prompt: &Path) -> Option<PathBuf> {
  let dir = zdotdir_base();
  if std::fs::create_dir_all(&dir).is_err() {
    return None;
  }
  let zshenv = "[[ -f /etc/zsh/zshenv ]] && source /etc/zsh/zshenv\n\
[[ -n \"$TONTOO_REALHOME\" && -f \"$TONTOO_REALHOME/.zshenv\" ]] && source \"$TONTOO_REALHOME/.zshenv\"\n";
  let quoted = sh_quote(prompt);
  let zshrc = format!(
    "[[ -f /etc/zsh/zshrc ]] && source /etc/zsh/zshrc\n[[ -n \"$TONTOO_REALHOME\" && -f \"$TONTOO_REALHOME/.zshrc\" ]] && source \"$TONTOO_REALHOME/.zshrc\"\n[[ -f {quoted} ]] && source {quoted}\n"
  );
  if std::fs::write(dir.join(".zshenv"), zshenv).is_err() {
    return None;
  }
  if std::fs::write(dir.join(".zshrc"), zshrc).is_err() {
    return None;
  }
  Some(dir)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn prompt_file_defines_green_user_at_host_prompt() {
    let content = include_str!("../Resources/tontoo-prompt.zsh");
    assert!(content.contains("%n@%m"));
    assert!(content.contains("%F{2}"));
    assert!(content.contains("%#"));
    assert!(content.contains("PROMPT="));
  }

  #[test]
  fn prompt_hook_enforces_format_after_themes() {
    let content = include_str!("../Resources/tontoo-prompt.zsh");
    assert!(content.contains("add-zsh-hook precmd _tontoo_prompt"));
    assert!(content.contains("unset RPROMPT"));
  }

  #[test]
  fn prompt_file_reports_cwd_via_osc7() {
    let content = include_str!("../Resources/tontoo-prompt.zsh");
    assert!(content.contains("]7;"));
    assert!(content.contains("add-zsh-hook"));
  }

  #[test]
  fn zdotdir_sources_real_config_before_prompt() {
    let fake = PathBuf::from("/nonexistent/tontoo-prompt.zsh");
    let Some(dir) = prepare_zdotdir(&fake) else {
      return;
    };
    let zshrc = std::fs::read_to_string(dir.join(".zshrc")).unwrap();
    let real = zshrc.find("TONTOO_REALHOME/.zshrc").unwrap();
    let prompt = zshrc.find(PROMPT_FILE).unwrap();
    assert!(real < prompt);
  }
}
