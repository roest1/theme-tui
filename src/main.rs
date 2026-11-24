//! The `theme` binary — a thin entry point.
//!
//! Everything of substance is in the library beside this, so the `dots`
//! console can push the same screens rather than growing its own copy. What is
//! left here is argument handling and the one process-level concern below.

use theme::{conf, show};
use tui_core::app;

fn main() -> std::io::Result<()> {
    // Rust ignores SIGPIPE and turns the resulting write error into a PANIC, so
    // `theme show | head` printed a backtrace instead of stopping. Every
    // other command-line tool on the machine dies quietly there; restoring the
    // default handler is what makes this one behave the same.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    match std::env::args().nth(1).as_deref() {
        Some("show") => {
            // Exits non-zero when a lane names a family this machine does not
            // have. status_fonts (lib/status.sh) counts drift on that and on
            // nothing else here -- see the note on the exit code in show.rs.
            std::process::exit(show::run());
        }
        Some("reset") => {
            conf::reset();
            println!("Fonts and colors reset to the defaults wezterm.lua and nvim ship");
        }
        Some("-h" | "--help") => {
            println!("theme          pick the font and the colors for each of wezterm's lanes");
            println!("theme show     what is set now, and whether wezterm honors it");
            println!("theme reset    back to the defaults wezterm.lua and nvim ship");
            println!();
            println!("Fonts and colors are two tabs: k off the top row reaches them.");
            println!("Browsing Google's fonts is inside the picker: open a lane,");
            println!("then the last row under it. The screen explains its own keys.");
            println!();
            println!("in the picker: j/k move · h/l close/open · gg/G ends");
            println!("               ctrl-d/u page · enter set · q quit");
        }
        _ => {
            app::run(Box::new(theme::screen()))?;
        }
    }
    Ok(())
}
