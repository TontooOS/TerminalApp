//! Pseudo terminal: spawns the shell on a PTY, reads its output on a
//! background thread and forwards keyboard input back.
//!
//! The master side is non blocking and polled from the render loop, so
//! program output never blocks the UI thread. The reader thread owns
//! the blocking read and hands chunks over a channel; both sides close
//! on EOF, which is how `exit` ends the window.

use std::io::Read;
use std::os::fd::FromRawFd;
use std::os::fd::RawFd;
use std::sync::mpsc::{Receiver, TryRecvError, channel};


/// Message from the reader thread.
pub enum Output {
  /// Raw bytes from the shell.
  Data(Vec<u8>),
  /// The shell exited with this status (`128 + signal` when killed).
  Exited(i32),
}

/// Errors that stop the terminal from starting.
#[derive(Debug)]
pub struct PtyError(pub String);

impl std::fmt::Display for PtyError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.0)
  }
}

impl std::error::Error for PtyError {}

/// A running shell on a pseudo terminal.
pub struct Pty {
  master: RawFd,
  reader: Receiver<Output>,
  exited: bool,
  status: Option<i32>,
}

impl Pty {
  /// Spawn `shell` with `args` on a PTY sized `cols` x `rows`, running
  /// in `cwd`. `env` holds `KEY=VALUE` strings merged over the current
  /// environment.
  ///
  /// Everything the child needs is built before `fork`, because only
  /// async-signal-safe work may follow it in a multithreaded process:
  /// the child just re-points its standard streams, changes directory
  /// and `execve`s.
  pub fn spawn(
    shell: &str,
    args: &[&str],
    cwd: &str,
    cols: u16,
    rows: u16,
    env: &[(String, String)],
  ) -> Result<Self, PtyError> {
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    size.ws_col = cols.max(1);
    size.ws_row = rows.max(1);
    let result = unsafe {
      libc::openpty(
        &mut master,
        &mut slave,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        &size,
      )
    };
    if result != 0 {
      return Err(PtyError(format!(
        "openpty failed: {}",
        std::io::Error::last_os_error()
      )));
    }
    // Close on exec: the shell keeps the slave only through its own
    // stdio, and no other child of this process may hold the master or
    // the slave (a stray copy would keep the reader from ever seeing
    // the shell exit).
    set_cloexec(master);
    set_cloexec(slave);

    let program = std::ffi::CString::new(resolve_program(shell)).map_err(|_| {
      PtyError(format!("shell path contains a NUL byte: {shell}"))
    })?;
    let directory = std::ffi::CString::new(cwd)
      .map_err(|_| PtyError(format!("working directory contains a NUL byte: {cwd}")))?;
    let mut owned: Vec<std::ffi::CString> = vec![program.clone()];
    for arg in args {
      owned.push(
        std::ffi::CString::new(*arg)
          .map_err(|_| PtyError(format!("argument contains a NUL byte: {arg}")))?,
      );
    }
    let environment = build_environment(env);
    let mut argv: Vec<*const libc::c_char> = owned.iter().map(|value| value.as_ptr()).collect();
    argv.push(std::ptr::null());
    let mut envp: Vec<*const libc::c_char> =
      environment.iter().map(|value| value.as_ptr()).collect();
    envp.push(std::ptr::null());

    // The child gets its own session so the shell owns the terminal:
    // Ctrl+C then reaches the foreground process group only.
    let pid = unsafe { libc::fork() };
    match pid {
      -1 => {
        let error = std::io::Error::last_os_error();
        unsafe {
          libc::close(master);
          libc::close(slave);
        }
        return Err(PtyError(format!("fork failed: {error}")));
      }
      0 => {}
      pid => {
        unsafe {
          libc::close(slave);
        }
        return Ok(Self {
          master,
          reader: spawn_reader(master, pid),
          exited: false,
          status: None,
        });
      }
    }

    unsafe {
      libc::setsid();
      if libc::ioctl(slave, libc::TIOCSCTTY, 0) != 0 {
        libc::_exit(127);
      }
      if libc::chdir(directory.as_ptr()) != 0 {
        libc::_exit(127);
      }
      restore_signals();
      libc::dup2(slave, libc::STDIN_FILENO);
      libc::dup2(slave, libc::STDOUT_FILENO);
      libc::dup2(slave, libc::STDERR_FILENO);
      if slave > libc::STDERR_FILENO {
        libc::close(slave);
      }
      libc::execve(program.as_ptr(), argv.as_ptr(), envp.as_ptr());
      libc::_exit(127);
    }
  }

  /// Read whatever the shell produced since the last call. `None` when
  /// nothing is pending.
  pub fn read_output(&mut self) -> Option<Output> {
    match self.reader.try_recv() {
      Ok(output) => {
        if let Output::Exited(status) = output {
          self.exited = true;
          self.status = Some(status);
        }
        Some(output)
      }
      Err(TryRecvError::Empty) => None,
      Err(TryRecvError::Disconnected) => None,
    }
  }

  /// True once the shell exited.
  pub fn exited(&self) -> bool {
    self.exited
  }

  /// Exit code of the shell, or `None` while it runs. A killed shell
  /// reports `128 + signal`.
  pub fn status(&self) -> Option<i32> {
    self.status
  }

  /// Send input to the shell.
  pub fn write(&mut self, bytes: &[u8]) -> bool {
    if bytes.is_empty() {
      return true;
    }
    let mut written = 0;
    while written < bytes.len() {
      let result = unsafe {
        libc::write(
          self.master,
          bytes[written..].as_ptr() as *const libc::c_void,
          bytes.len() - written,
        )
      };
      if result > 0 {
        written += result as usize;
        continue;
      }
      if result < 0 {
        let error = std::io::Error::last_os_error();
        let again = matches!(
          error.raw_os_error(),
          Some(libc::EAGAIN) | Some(libc::EINTR)
        );
        if again {
          std::thread::sleep(std::time::Duration::from_millis(1));
          continue;
        }
      }
      return false;
    }
    true
  }

  /// Tell the shell the window changed size (SIGWINCH follows).
  pub fn resize(&mut self, cols: u16, rows: u16) {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    size.ws_col = cols.max(1);
    size.ws_row = rows.max(1);
    unsafe {
      libc::ioctl(self.master, libc::TIOCSWINSZ, &size);
    }
  }

}

/// Mark a descriptor close on exec.
fn set_cloexec(fd: libc::c_int) {
  unsafe {
    let flags = libc::fcntl(fd, libc::F_GETFD);
    if flags >= 0 {
      libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
    }
  }
}

/// Reader thread: blocking reads from the master fd, chunks sent over a
/// channel. Closes on EOF or `EIO`, which is what a PTY master reports
/// once the child is gone; the real exit status comes from `waitpid`.
fn spawn_reader(master: RawFd, pid: libc::pid_t) -> Receiver<Output> {
  let (sender, receiver) = channel();
  let duplicated = unsafe { libc::fcntl(master, libc::F_DUPFD_CLOEXEC, 0) };
  if duplicated < 0 {
    let _ = sender.send(Output::Exited(1));
    return receiver;
  }
  std::thread::spawn(move || {
    let mut file = unsafe { std::fs::File::from_raw_fd(duplicated) };
    let mut buffer = [0u8; 8192];
    let mut done = false;
    while !done {
      match file.read(&mut buffer) {
        Ok(0) => done = true,
        Ok(count) => {
          let chunk = buffer[..count].to_vec();
          if sender.send(Output::Data(chunk)).is_err() {
            return;
          }
        }
        Err(error) => {
          match error.raw_os_error() {
            Some(libc::EINTR) => {}
            Some(libc::EIO) | Some(libc::EBADF) | _ => done = true,
          }
        }
      }
    }
    let _ = sender.send(Output::Exited(exit_status(pid)));
  });
  receiver
}

/// Wait for the child and turn its state into an exit code: the shell
/// status, or `128 + signal` when it was killed.
fn exit_status(pid: libc::pid_t) -> i32 {
  let mut status: libc::c_int = 0;
  let result = unsafe { libc::waitpid(pid, &mut status, 0) };
  if result <= 0 {
    return 0;
  }
  if libc::WIFEXITED(status) {
    return libc::WEXITSTATUS(status);
  }
  if libc::WIFSIGNALED(status) {
    return 128 + libc::WTERMSIG(status);
  }
  0
}

/// Absolute path of the shell program. A bare name is looked up in the
/// usual binary directories and then in `PATH`, so `execve` gets an
/// absolute path and the child never has to search.
fn resolve_program(shell: &str) -> String {
  if shell.contains('/') {
    return shell.to_string();
  }
  for dir in ["/bin", "/usr/bin", "/usr/local/bin"] {
    let candidate = std::path::Path::new(dir).join(shell);
    if candidate.is_file() {
      return candidate.display().to_string();
    }
  }
  if let Some(path) = std::env::var_os("PATH") {
    for dir in std::env::split_paths(&path) {
      let candidate = dir.join(shell);
      if candidate.is_file() {
        return candidate.display().to_string();
      }
    }
  }
  shell.to_string()
}

/// The child environment: the inherited variables plus the extra pairs,
/// and `TERM` / `COLORTERM` for full color support.
fn build_environment(extra: &[(String, String)]) -> Vec<std::ffi::CString> {
  let mut values: Vec<(String, String)> = std::env::vars().collect();
  for (key, value) in extra {
    match values.iter_mut().find(|(existing, _)| existing == key) {
      Some(slot) => slot.1 = value.clone(),
      None => values.push((key.clone(), value.clone())),
    }
  }
  if !values.iter().any(|(key, _)| key == "TERM") {
    values.push(("TERM".to_string(), "xterm-256color".to_string()));
  }
  if !values.iter().any(|(key, _)| key == "COLORTERM") {
    values.push(("COLORTERM".to_string(), "truecolor".to_string()));
  }
  values
    .into_iter()
    .filter_map(|(key, value)| {
      let entry = format!("{key}={value}");
      std::ffi::CString::new(entry).ok()
    })
    .collect()
}

impl Drop for Pty {
  fn drop(&mut self) {
    if self.master >= 0 {
      unsafe {
        libc::close(self.master);
      }
      self.master = -1;
    }
  }
}

/// Reset the signal disposition of the child: the shell must not inherit
/// a blocked or ignored terminal signal from the window process.
fn restore_signals() {
  unsafe {
    for signal in [libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGCHLD] {
      libc::signal(signal, libc::SIG_DFL);
    }
    // A reader on the PTY must not die on SIGPIPE when the shell exits.
    libc::signal(libc::SIGPIPE, libc::SIG_IGN);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn spawn_runs_a_shell_and_reports_output() {
    let (cols, rows) = (80u16, 24u16);
    let mut pty = Pty::spawn("/bin/sh", &["-c", "printf 'hello\n'; exit 7"], "/tmp", cols, rows, &[])
      .expect("spawn shell");
    let mut text = String::new();
    let mut status = None;
    for _ in 0..200 {
      match pty.read_output() {
        Some(Output::Data(bytes)) => text.push_str(&String::from_utf8_lossy(&bytes)),
        Some(Output::Exited(code)) => {
          status = Some(code);
          break;
        }
        None => std::thread::sleep(std::time::Duration::from_millis(10)),
      }
    }
    assert_eq!(text.trim_end(), "hello", "exit status was {status:?}");
    assert_eq!(status, Some(7), "shell exit status reaches the app");
    assert!(pty.exited());
  }

  #[test]
  fn input_reaches_the_shell() {
    let mut pty = Pty::spawn("/bin/cat", &[], "/tmp", 80, 24, &[]).expect("spawn cat");
    assert!(pty.write(b"ping\n"));
    let mut text = String::new();
    for _ in 0..200 {
      match pty.read_output() {
        Some(Output::Data(bytes)) => {
          text.push_str(&String::from_utf8_lossy(&bytes));
          if text.contains("ping") {
            break;
          }
        }
        Some(Output::Exited(_)) => break,
        None => std::thread::sleep(std::time::Duration::from_millis(10)),
      }
    }
    assert!(text.contains("ping"), "got {text:?}");
  }

  #[test]
  fn extra_environment_reaches_the_shell() {
    let extra = vec![("TONTOO_TEST_VALUE".to_string(), "42".to_string())];
    let mut pty = Pty::spawn(
      "/bin/sh",
      &["-c", "printf '%s\n' \"$TONTOO_TEST_VALUE\""],
      "/tmp",
      80,
      24,
      &extra,
    )
    .expect("spawn shell");
    let mut text = String::new();
    for _ in 0..200 {
      match pty.read_output() {
        Some(Output::Data(bytes)) => {
          text.push_str(&String::from_utf8_lossy(&bytes));
          if text.len() >= 2 {
            break;
          }
        }
        Some(Output::Exited(_)) => break,
        None => std::thread::sleep(std::time::Duration::from_millis(10)),
      }
    }
    assert_eq!(text.trim_end(), "42", "the line discipline ends lines with CRLF");
  }

  #[test]
  fn missing_shell_reports_failure() {
    let result = Pty::spawn("/definitely/not/here", &[], "/tmp", 80, 24, &[]);
    // openpty still succeeds, the shell itself fails: the child exits
    // with 127 and the reader reports it.
    if let Ok(mut pty) = result {
      let mut status = None;
      for _ in 0..200 {
        match pty.read_output() {
          Some(Output::Exited(code)) => {
            status = Some(code);
            break;
          }
          Some(_) => {}
          None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
      }
      assert_eq!(status, Some(127));
    }
  }
}