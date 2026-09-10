//! Terminal: macOS-style VTE terminal for TontooOS.
//!
//! Mac decoration bar (`WindowType::Mac`) with a live title (OSC program
//! title, else current path). Transparent background (`0.85` + `20px`
//! blur), SF Mono font, full ANSI colors, audible + visual bell. Default
//! shell is `zsh`.

mod config;
mod lang;
mod prompt;
mod terminal;

sdk::preinclude!();

use UIKit::prelude::*;

struct TerminalDelegate;

impl AppDelegate for TerminalDelegate {
  fn view(&self) -> Box<dyn Widget> {
    Box::new(terminal::TerminalRoot::new())
  }
}

fn main() {
  lang::init();
  let mut app = App::with_delegate(lang::t("app.title"), 900, 600, TerminalDelegate);
  app.set_window_type(WindowType::Mac);
  // Medium glass: transparent background plus backdrop blur.
  app.set_window_transparency(0.85);
  app.set_window_blur(20.0);
  app.auto_color_scheme();
  app.run();
}
