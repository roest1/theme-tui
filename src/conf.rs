//! `~/.config/dotfiles/theme.conf`: the one file every pick is written to.
//!
//! Keyed by lane, then by what is being set:
//!
//! ```text
//! shell.font         = 0xProto Nerd Font
//! shell.colors       = dracula
//! nvim.editor.colors = onedark
//! ```
//!
//! Two programs read it. wezterm.lua takes the fonts and the terminal's own
//! colors, and nvim's `external/theme.lua` takes the nvim colors. Each skips a
//! line it cannot read and ignores a key it does not know, and this file keeps
//! its half of that bargain: a key it does not know is WRITTEN BACK, so a
//! reader can gain one before the picker learns to set it.
//!
//! Two screens write it, the fonts and the colors, each holding only its own
//! picks. So a write is never "this is the file now": it reads the file, sets
//! what changed and writes the result. See [`update`].

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::fonts;
use crate::lanes::LANES;

/// Every key in the file and what it is set to.
pub type Conf = BTreeMap<String, String>;

/// The keys this program sets, in the order the file lists them. Anything else
/// in the file follows, sorted.
const KEYS: [&str; 10] = [
    "shell.font",
    "shell.colors",
    "shell.opacity",
    "nvim.ui.font",
    "nvim.ui.colors",
    "nvim.ui.transparent",
    "nvim.editor.font",
    "nvim.editor.colors",
    "nvim.editor.transparent",
    "claude.font",
];

pub fn path() -> PathBuf {
    fonts::xdg("XDG_CONFIG_HOME", ".config")
        .join("dotfiles")
        .join("theme.conf")
}

/// Where the fonts were kept until they moved in here, keyed by the bare lane.
fn old_fonts_path() -> PathBuf {
    fonts::wezterm_dir().join("fonts.conf")
}

/// The `key = value` lines of a file, in order.
///
/// The same rule as the Lua readers' pattern, `^%s*([%a_][%w_.]*)%s*=%s*(.-)%s*$`
/// with an empty value refused: what this accepts and what they accept has to
/// be one set of lines, or a pick could be written that a reader then skips.
pub fn parse(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let (key, value) = (key.trim(), value.trim());
            let mut chars = key.chars();
            let head = chars.next()?;
            let named = (head.is_ascii_alphabetic() || head == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
            (named && !value.is_empty()).then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

/// One `Conf` from the file and the two spellings that came before it.
///
/// `old_fonts` is `fonts.conf`, where `shell = Family` meant the font: it
/// becomes `shell.font`, underneath anything the file itself says.
///
/// A bare `nvim.ui = scheme` in the file is from the week it was nvim's
/// alone, when the only thing to set was a colorscheme: it becomes
/// `nvim.ui.colors`, underneath a `nvim.ui.colors` line. Both are folded in
/// here, so the first write puts everything in the one spelling.
pub fn merged(theme: &str, old_fonts: &str) -> Conf {
    let mut conf = Conf::new();
    for (lane, family) in parse(old_fonts) {
        if LANES.contains(&lane.as_str()) {
            conf.insert(format!("{lane}.font"), family);
        }
    }
    // The bare keys first, so a line in the new spelling lands on top of one
    // wherever in the file it is.
    let theirs = parse(theme);
    let bare = |key: &str| key == "nvim.ui" || key == "nvim.editor";
    for (key, value) in theirs.iter().filter(|(key, _)| bare(key)) {
        conf.insert(format!("{key}.colors"), value.clone());
    }
    for (key, value) in theirs.into_iter().filter(|(key, _)| !bare(key)) {
        conf.insert(key, value);
    }
    conf
}

pub fn load() -> Conf {
    merged(
        &fs::read_to_string(path()).unwrap_or_default(),
        &fs::read_to_string(old_fonts_path()).unwrap_or_default(),
    )
}

/// The file's text for `conf`.
pub fn render(conf: &Conf) -> String {
    let mut body = String::from(
        "# Written by `font`. Machine-local: not in git, not linked, not in\n\
         # deps.conf — same footing as ~/.bash_local.\n\
         # Read by wezterm/wezterm.lua and nvim/lua/external/theme.lua. Delete this\n\
         # file to get the defaults.\n",
    );
    let known = KEYS
        .iter()
        .filter_map(|key| Some((*key, conf.get(*key)?.as_str())));
    let other = conf
        .iter()
        .filter(|(key, _)| !KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.as_str(), value.as_str()));
    for (key, value) in known.chain(other) {
        body.push_str(&format!("{key} = {value}\n"));
    }
    body
}

/// Set (`Some`) or take out (`None`) each key, and write the file.
///
/// Read, changed and written in one go, because the other screen may have
/// written since this one last looked.
///
/// Truncate-and-write, deliberately, NOT write-to-temp-and-rename. wezterm
/// watches this path via add_to_config_reload_watch_list, and an atomic rename
/// swaps the inode out from under that watch: the repaint may simply never
/// happen. Rewriting in place keeps the inode the watcher is holding. The
/// torn-write window that buys back is harmless: both readers take the file a
/// line at a time and skip anything malformed, and nvim waits for the writes
/// to stop before it reads.
///
/// The fonts' old file goes once the new one is written. Left behind, it would
/// be read underneath by wezterm, and `font reset` would fall back to it and
/// not to the defaults.
pub fn update(changes: &[(&str, Option<&str>)]) -> io::Result<Conf> {
    let mut conf = load();
    for (key, value) in changes {
        match value {
            Some(value) => conf.insert((*key).to_string(), (*value).to_string()),
            None => conf.remove(*key),
        };
    }
    let path = path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, render(&conf))?;
    let _ = fs::remove_file(old_fonts_path());
    Ok(conf)
}

/// Back to what the readers ship: no file, in either spelling.
pub fn reset() {
    let _ = fs::remove_file(path());
    let _ = fs::remove_file(old_fonts_path());
}

#[cfg(test)]
mod tests {
    use super::{merged, parse, render};

    fn pairs(text: &str) -> Vec<(String, String)> {
        parse(text)
    }

    /// The Lua readers skip these lines, so the writer must not count them.
    #[test]
    fn only_a_line_the_readers_take_is_a_pick() {
        let got = pairs(
            "# a comment = not a key\n\
             shell.font = 0xProto Nerd Font\n\
             \x20 nvim.ui.colors=dracula  \n\
             empty.value =\n\
             9lives = no\n\
             has space = no\n\
             a-dash = no\n\
             no equals sign\n\
             _private = yes\n\
             eq = a = b\n",
        );
        assert_eq!(
            got,
            [
                ("shell.font", "0xProto Nerd Font"),
                ("nvim.ui.colors", "dracula"),
                ("_private", "yes"),
                ("eq", "a = b"),
            ]
            .map(|(k, v)| (k.to_string(), v.to_string()))
        );
    }

    #[test]
    fn the_fonts_old_file_moves_in_underneath() {
        let conf = merged(
            "shell.font = Hack\n",
            "shell = Fira Code\nnvim.editor = Iosevka\nnot.a.lane = X\n",
        );
        assert_eq!(conf["shell.font"], "Hack", "the new file wins");
        assert_eq!(conf["nvim.editor.font"], "Iosevka");
        assert!(!conf.contains_key("not.a.lane.font"));
        assert!(!conf.contains_key("nvim.editor"), "the bare key is gone");
    }

    /// In theme.conf a bare `nvim.ui` was a colorscheme, never a font.
    #[test]
    fn a_bare_nvim_lane_in_the_file_is_its_colorscheme() {
        let conf = merged("nvim.ui = dracula\nnvim.editor = onedark\n", "");
        assert_eq!(conf["nvim.ui.colors"], "dracula");
        assert_eq!(conf["nvim.editor.colors"], "onedark");
        assert!(!conf.contains_key("nvim.ui") && !conf.contains_key("nvim.ui.font"));

        let both = merged("nvim.ui.colors = onedark\nnvim.ui = dracula\n", "");
        assert_eq!(both["nvim.ui.colors"], "onedark", "the new key wins");
    }

    /// A key a reader gained before this program learned to set it.
    #[test]
    fn a_key_nobody_here_knows_is_written_back() {
        let conf = merged(
            "icons = material\nclaude.font = Hack\nshell.colors = dracula\nshell.font = Hack\n",
            "",
        );
        let text = render(&conf);
        let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            lines,
            [
                "shell.font = Hack",
                "shell.colors = dracula",
                "claude.font = Hack",
                "icons = material",
            ]
        );
        assert_eq!(merged(&text, ""), conf, "and it reads back as written");
    }
}
