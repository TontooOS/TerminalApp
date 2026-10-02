//! Terminal for TontooOS, built with TontooUI.
//!
//! macOS style window: a `Titlebar` with traffic lights and a live
//! title (the running program via OSC 0/1/2, otherwise the folder via
//! OSC 7), and below it a terminal grid drawn straight into the Vello
//! scene. The grid is driven by an own VT parser over a PTY, so the ISO
//! needs no `vte4` package and the window has no background blur: the
//! body is the flat TontooOS background token (`#1b2022` dark,
//! `#ffffff` light). Default shell is `zsh`.

mod app;
mod clipboard;
mod config;
mod grid;
mod input;
mod lang;
mod parser;
mod prompt;
mod pty;
mod render;
mod theme;

sdk::preinclude!();

use TontooUI::renderer::window::run;

fn main() {
  lang::init();
  let app = app::TerminalApp::new();
  if let Err(error) = run(
    &lang::t("app.title"),
    config::WINDOW_WIDTH,
    config::WINDOW_HEIGHT,
    app,
  ) {
    eprintln!("terminal: {error}");
    std::process::exit(1);
  }
}