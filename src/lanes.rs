//! The four lanes, and what each is set to when nothing has been picked.
//!
//! Kept in step with DEFAULT_FONTS in wezterm/wezterm.lua, the default palette
//! there and DEFAULT in nvim's external/theme.lua by hand. They are spellings
//! of one fact and nothing holds them together, which is affordable only
//! because disagreeing is harmless: the readers ignore keys they do not know
//! and fall back to their own defaults for keys they do not get.

use crate::conf::{self, Conf};

pub const LANES: [&str; 4] = ["shell", "nvim.ui", "nvim.editor", "claude"];

/// The lanes that have colors of their own. `claude` is not one: a Claude pane
/// is a terminal pane, and wezterm colors a window, not a pane.
pub const COLOR_LANES: [&str; 3] = ["shell", "nvim.ui", "nvim.editor"];

/// The terminal's palette when `shell.colors` is not set.
pub const DEFAULT_PALETTE: &str = "rose-pine";

/// nvim's colorscheme when `nvim.ui.colors` is not set.
pub const DEFAULT_SCHEME: &str = "rose-pine-moon";

pub fn covers(lane: &str) -> &'static str {
    match lane {
        "shell" => "the shell — any pane that is not nvim or Claude",
        // Named for the whole of nvim's chrome, not for oil. Oil is the most
        // visible thing it draws; it is also telescope, the statusline, the
        // gutter and every float, because it is the window's base font while
        // nvim holds focus.
        "nvim.ui" => "nvim's chrome — oil, telescope, statusline, gutter",
        "nvim.editor" => "the file you are editing in nvim",
        _ => "Claude Code panes (pinned)",
    }
}

pub fn default_for(lane: &str) -> &'static str {
    if lane == "nvim.editor" {
        "JetBrainsMono Nerd Font"
    } else {
        "0xProto Nerd Font"
    }
}

/// A colorscheme that is not offered with a transparent background.
///
/// A taste, not a rule: nvim will load One Dark transparent if the file says
/// so. Over the terminal's own background it reads badly, which was tried and
/// rejected, so the picker does not put it there.
pub fn opaque_only(scheme: &str) -> bool {
    scheme == "onedark"
}

pub fn state_path() -> std::path::PathBuf {
    conf::path()
}

/// Each lane's font, in `LANES` order.
pub fn fonts_of(conf: &Conf) -> Vec<(String, String)> {
    LANES
        .iter()
        .map(|lane| {
            let family = conf
                .get(&format!("{lane}.font"))
                .cloned()
                .unwrap_or_else(|| default_for(lane).into());
            ((*lane).to_string(), family)
        })
        .collect()
}

pub fn read() -> Vec<(String, String)> {
    fonts_of(&conf::load())
}

/// Write the four fonts, and leave everything else in the file as it is.
pub fn write(state: &[(String, String)]) -> std::io::Result<()> {
    let keys: Vec<String> = state
        .iter()
        .map(|(lane, _)| format!("{lane}.font"))
        .collect();
    let changes: Vec<(&str, Option<&str>)> = keys
        .iter()
        .zip(state)
        .map(|(key, (_, family))| (key.as_str(), Some(family.as_str())))
        .collect();
    conf::update(&changes).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{default_for, fonts_of, LANES};
    use crate::conf::merged;

    #[test]
    fn a_lane_nothing_names_gets_its_default() {
        let fonts = fonts_of(&merged("nvim.ui.font = Hack\nshell.colors = dracula\n", ""));
        let lanes: Vec<&str> = fonts.iter().map(|(lane, _)| lane.as_str()).collect();
        assert_eq!(lanes, LANES);
        assert_eq!(fonts[1].1, "Hack");
        assert_eq!(fonts[0].1, default_for("shell"));
        assert_eq!(fonts[2].1, default_for("nvim.editor"));
    }
}
