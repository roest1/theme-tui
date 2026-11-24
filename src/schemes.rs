//! What there is to pick a color from, asked of whoever owns it.
//!
//! Two lists, and neither is written down here. The terminal's palettes are
//! the files in wezterm's `themes` directory, which wezterm.lua reads the
//! picked one of. nvim's colorschemes are whatever nvim says it has, with the
//! colors each one actually sets. A list kept in this crate would be a second
//! copy of both, wrong the first time a scheme is installed.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::conf;
use crate::fonts;

pub type Rgb = [u8; 3];

/// `#rrggbb` or `rrggbb`.
fn rgb(text: &str) -> Option<Rgb> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// One of wezterm's themes: the terminal's background and its tab bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub name: String,
    pub base: Rgb,
    pub tab_active: Rgb,
    pub accent: Rgb,
    pub surface: Rgb,
    pub overlay: Rgb,
    pub highlight_med: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub subtle: Rgb,
}

impl Palette {
    /// Whole or not at all, the rule wezterm.lua reads a theme by: a file
    /// missing a color, or holding something that is not `#rrggbb`, is no
    /// theme. Offering one wezterm would then refuse is a pick that does
    /// nothing.
    pub fn parse(name: &str, text: &str) -> Option<Self> {
        let pairs = conf::parse(text);
        let color = |key: &str| {
            let value = &pairs.iter().rev().find(|(k, _)| k == key)?.1;
            value.starts_with('#').then(|| rgb(value))?
        };
        Some(Self {
            name: name.to_string(),
            base: color("base")?,
            tab_active: color("tab_active")?,
            accent: color("accent")?,
            surface: color("surface")?,
            overlay: color("overlay")?,
            highlight_med: color("highlight_med")?,
            text: color("text")?,
            muted: color("muted")?,
            subtle: color("subtle")?,
        })
    }

    /// The nine colors with their names, in the order a file lists them.
    pub fn named(&self) -> [(&'static str, Rgb); 9] {
        [
            ("base", self.base),
            ("tab_active", self.tab_active),
            ("accent", self.accent),
            ("surface", self.surface),
            ("overlay", self.overlay),
            ("highlight_med", self.highlight_med),
            ("text", self.text),
            ("muted", self.muted),
            ("subtle", self.subtle),
        ]
    }
}

/// Every theme wezterm can be given, by name.
///
/// A name wezterm.lua would not take is left out too: it refuses anything but
/// letters, digits, `_` and `-`, so that a name can never be a path.
pub fn palettes() -> Vec<Palette> {
    let dir = fonts::wezterm_dir().join("themes");
    let mut out: Vec<Palette> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_stem()?.to_str()?;
            let named = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !named || path.extension()?.to_str()? != "conf" {
                return None;
            }
            Palette::parse(name, &std::fs::read_to_string(&path).ok()?)
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// One of nvim's colorschemes, as the colors of the groups a preview draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scheme {
    pub name: String,
    groups: HashMap<String, (Option<Rgb>, Option<Rgb>)>,
}

impl Scheme {
    pub fn fg(&self, group: &str) -> Option<Rgb> {
        self.groups.get(group)?.0
    }

    pub fn bg(&self, group: &str) -> Option<Rgb> {
        self.groups.get(group)?.1
    }
}

/// What `schemes.lua` printed. Anything that is not one of its lines is
/// skipped: a plugin is free to print at startup.
pub fn parse_schemes(out: &str) -> Vec<Scheme> {
    out.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            if fields.next()? != "scheme" {
                return None;
            }
            let name = fields.next().filter(|n| !n.is_empty())?.to_string();
            let groups = fields
                .filter_map(|field| {
                    let (group, colors) = field.split_once('=')?;
                    let (fg, bg) = colors.split_once(',')?;
                    Some((group.to_string(), (rgb(fg), rgb(bg))))
                })
                .collect();
            Some(Scheme { name, groups })
        })
        .collect()
}

const SCRIPT: &str = include_str!("schemes.lua");

/// How long nvim gets. It answers in a tenth of a second; a config that
/// stops to ask something at startup would otherwise never answer at all.
const PATIENCE: Duration = Duration::from_secs(10);

/// Ask nvim what colorschemes it has. Blocks, so it is called off the UI
/// thread. `Err` says why there is no list, in words for the pane.
///
/// The script goes in through the environment and is run with `loadstring`:
/// no file to write and clean up, and nothing to quote on a command line.
/// nvim is started WITH the user's config, because that is what puts the
/// plugins' schemes where nvim can find them.
pub fn ask_nvim() -> Result<Vec<Scheme>, String> {
    let mut child = Command::new("nvim")
        .env("THEME_TUI_LUA", SCRIPT)
        .args([
            "--headless",
            // No shada and no swap file: this nvim is here to answer a
            // question, and must not leave a trace in the editor's history.
            "-i",
            "NONE",
            "-n",
            "-c",
            "lua assert(loadstring(vim.env.THEME_TUI_LUA))()",
            "-c",
            "qa!",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "nvim is not installed".to_string(),
            _ => format!("could not start nvim: {e}"),
        })?;
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        let _ = stdout.read_to_string(&mut out);
        out
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < PATIENCE => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("nvim did not answer".to_string());
            }
        }
    }
    let schemes = parse_schemes(&reader.join().unwrap_or_default());
    if schemes.is_empty() {
        return Err("nvim has no colorschemes of its own installed".to_string());
    }
    Ok(schemes)
}

#[cfg(test)]
mod tests {
    use super::{parse_schemes, Palette};

    const WHOLE: &str = "# Dracula\n\
        base = #282a36\ntab_active = #191a21\naccent = #bd93f9\n\
        surface = #21222c\noverlay = #343746\nhighlight_med = #44475a\n\
        text = #f8f8f2\nmuted = #6272a4\nsubtle = #adb5cb\n";

    #[test]
    fn a_theme_is_nine_colors() {
        let p = Palette::parse("dracula", WHOLE).expect("a whole theme");
        assert_eq!(p.name, "dracula");
        assert_eq!(p.base, [0x28, 0x2a, 0x36]);
        assert_eq!(p.subtle, [0xad, 0xb5, 0xcb]);
        assert_eq!(p.named().len(), 9);
    }

    /// wezterm.lua refuses these, so offering them would be a pick that
    /// changes nothing.
    #[test]
    fn a_theme_wezterm_would_refuse_is_not_offered() {
        let missing = WHOLE.replace("subtle = #adb5cb\n", "");
        assert_eq!(Palette::parse("x", &missing), None, "a color short");
        for bad in ["black", "#fff", "#12345g", "282a36", "#282a3600"] {
            let text = WHOLE.replace("base = #282a36", &format!("base = {bad}"));
            assert_eq!(Palette::parse("x", &text), None, "base = {bad}");
        }
    }

    #[test]
    fn nvim_is_read_by_its_marked_lines_and_nothing_else() {
        let out = "a plugin said hello\n\
            scheme\tdracula\tNormal=f8f8f2,282a36\tComment=6272a4,\tVisual=,44475a\tBroken\n\
            scheme\t\tNormal=ffffff,000000\n\
            scheme\tbare\n";
        let schemes = parse_schemes(out);
        let names: Vec<&str> = schemes.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["dracula", "bare"]);
        let d = &schemes[0];
        assert_eq!(d.fg("Normal"), Some([0xf8, 0xf8, 0xf2]));
        assert_eq!(d.bg("Normal"), Some([0x28, 0x2a, 0x36]));
        assert_eq!(d.bg("Comment"), None, "a group with no background");
        assert_eq!(d.fg("Visual"), None);
        assert_eq!(d.fg("NoSuchGroup"), None);
        assert_eq!(schemes[1].fg("Normal"), None, "a scheme that said nothing");
    }
}
