//! The icons screen — which set draws a file's icon.
//!
//! One pick for the terminal and not one per lane: an icon is a character,
//! and `ls`, nvim and the prompt all print into the same window.
//!
//! The preview is TEXT, where the fonts screen draws an image, and that is on
//! purpose. A specimen is rasterized here because a font can be shown without
//! being set. An icon cannot: it is a character in a font the TERMINAL has,
//! drawn by whichever machine the window is on. So the pane prints the real
//! characters and the terminal draws them, or fails to, and either way it is
//! a picture of what `ls` will look like and not of what this process hopes.
//!
//! Three readers take the pick, none of them this crate: dotfiles' wezterm.lua
//! has the font in every fallback chain whether or not it is picked (which is
//! what lets this pane draw the set before it is), nvim swaps what oil and
//! telescope are handed, and the shell points eza at a theme built from the
//! same table. This screen writes one key and knows none of that.
//!
//! Two sets. Nerd Font is what there has always been and what the file means
//! by saying nothing. Material Icon Theme is the color one, and nothing about
//! it is written down here: which code point is which icon, and which file
//! gets which, is read out of the table that ships beside the font. A code
//! point only holds for one upstream release (roest1/fonts, mkicons.py).

use std::collections::HashMap;
use std::path::Path;

use tui_core::{nav::Action, width, Flow, Pane, Row, Screen};

use crate::conf::{self, Conf};
use crate::fonts;
use crate::show::tildify;

/// The key in theme.conf. No lane in front of it: see the top of this file.
pub const KEY: &str = "icons";

const FONT_FILE: &str = "MaterialIconTheme.ttf";
const TABLE_FILE: &str = "material-icons.tsv";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Set {
    Nerd,
    Material,
}

/// In the order the rows are drawn.
const SETS: [Set; 2] = [Set::Nerd, Set::Material];

/// What the file says. `material` is the one value that means anything;
/// every other, and no key at all, is the default.
fn now(conf: &Conf) -> Set {
    match conf.get(KEY).map(String::as_str) {
        Some("material") => Set::Material,
        _ => Set::Nerd,
    }
}

/// The set in force, for `theme show`.
pub fn summary(conf: &Conf) -> &'static str {
    match now(conf) {
        Set::Nerd => "nerd font",
        Set::Material => "material",
    }
}

// ─── Material's table ───────────────────────────────────────────────────────

/// `material-icons.tsv`: every icon and its code point, and upstream's own
/// word on which icon a file or a folder gets.
#[derive(Debug, Default)]
pub struct Material {
    /// "5.40.0", out of the file's first line. Empty when it does not say.
    pub version: String,
    /// Every icon, in code point order.
    icons: Vec<(String, char)>,
    by_name: HashMap<String, char>,
    ext: HashMap<String, String>,
    file: HashMap<String, String>,
    folder: HashMap<String, String>,
    default_file: String,
    default_folder: String,
}

impl Material {
    /// `None` for a file with no icon in it, which is no table.
    pub fn parse(text: &str) -> Option<Self> {
        let mut me = Self::default();
        for line in text.lines() {
            if let Some(comment) = line.strip_prefix('#') {
                if let Some(rest) = comment.trim().strip_prefix("Material Icon Theme ") {
                    if me.version.is_empty() {
                        me.version = rest
                            .chars()
                            .take_while(|c| c.is_ascii_digit() || *c == '.')
                            .collect();
                    }
                }
                continue;
            }
            let mut fields = line.split('\t');
            let (Some(kind), Some(key), Some(value)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let (key, value) = (key.to_string(), value.to_string());
            match kind {
                "icon" => {
                    let Some(c) = u32::from_str_radix(&value, 16)
                        .ok()
                        .and_then(char::from_u32)
                    else {
                        continue;
                    };
                    me.by_name.insert(key.clone(), c);
                    me.icons.push((key, c));
                }
                "ext" => {
                    me.ext.insert(key, value);
                }
                "file" => {
                    me.file.insert(key.to_lowercase(), value);
                }
                "folder" => {
                    me.folder.insert(key.to_lowercase(), value);
                }
                "default" if key == "file" => me.default_file = value,
                "default" if key == "folder" => me.default_folder = value,
                _ => {}
            }
        }
        me.icons.sort_by_key(|(_, c)| *c);
        (!me.icons.is_empty()).then_some(me)
    }

    /// The table in `dir`, if the font it describes is there with it. `Err`
    /// says what is missing, in words for the pane.
    fn load_from(dir: &Path) -> Result<Self, String> {
        let at = tildify(&dir.display().to_string());
        if !dir.join(FONT_FILE).is_file() {
            return Err(format!("{FONT_FILE} is not in {at}"));
        }
        let text = std::fs::read_to_string(dir.join(TABLE_FILE))
            .map_err(|_| format!("{TABLE_FILE} is not in {at}, beside the font"))?;
        Self::parse(&text).ok_or_else(|| format!("{TABLE_FILE} lists no icons"))
    }

    /// The icon `name` gets: its whole name first, then its extension, longest
    /// first (`d.ts` before `ts`), then what anything gets. The same order as
    /// the editor extension the table comes from.
    pub fn icon_for(&self, name: &str, dir: bool) -> Option<char> {
        let lower = name.to_lowercase();
        let icon = if dir {
            self.folder.get(&lower).unwrap_or(&self.default_folder)
        } else {
            self.file
                .get(&lower)
                .or_else(|| {
                    lower
                        .match_indices('.')
                        .find_map(|(dot, _)| self.ext.get(&lower[dot + 1..]))
                })
                .unwrap_or(&self.default_file)
        };
        self.by_name.get(icon).copied()
    }
}

// ─── the screen ─────────────────────────────────────────────────────────────

pub struct IconScreen {
    conf: Conf,
    /// The table, or why Material cannot be offered.
    material: Result<Material, String>,
    /// Whether this is a terminal known to be given the icon font.
    wezterm: bool,
    status: String,
}

impl Default for IconScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl IconScreen {
    pub fn new() -> Self {
        Self::with(
            conf::load(),
            Material::load_from(&fonts::config_font_dir()),
            std::env::var("TERM_PROGRAM").as_deref() == Ok("WezTerm"),
        )
    }

    fn with(conf: Conf, material: Result<Material, String>, wezterm: bool) -> Self {
        Self {
            conf,
            material,
            wezterm,
            status: String::new(),
        }
    }

    /// What `enter` on `set` writes to the key (`None` takes it out), and
    /// what to say once it has. `Err` is a set that will not be picked.
    ///
    /// The default is the key LEFT OUT, never `icons = nerd`: a file that
    /// says nothing is how every machine starts, and one spelling of that is
    /// enough.
    fn pick(&self, set: Set) -> Result<(Option<&'static str>, String), String> {
        match (set, &self.material) {
            (Set::Nerd, _) => Ok((None, "icons \u{2192} nerd font".into())),
            (Set::Material, Ok(_)) => Ok((Some("material"), "icons \u{2192} material".into())),
            // Its characters are private-use: with no font they are boxes.
            (Set::Material, Err(why)) => Err(format!("material is not installed: {why}")),
        }
    }
}

// ─── drawing ────────────────────────────────────────────────────────────────

/// A directory listing small enough to read at once, and the glyph eza gives
/// each name (0.23.5, asked once: `eza --icons=always -1`).
///
/// A copy of someone else's table, which this crate otherwise refuses to
/// keep, and it is here only because it is a PICTURE: nothing reads it to
/// decide anything, and if eza moves a glyph the worst it can be is a preview
/// one glyph out of date. Material's side of the same listing is looked up.
const LISTING: [(&str, bool, char); 24] = [
    ("src", true, '\u{f08de}'),
    (".github", true, '\u{e5fd}'),
    ("node_modules", true, '\u{e5fa}'),
    ("docs", true, '\u{e5ff}'),
    ("Cargo.toml", false, '\u{e68b}'),
    ("main.rs", false, '\u{e68b}'),
    ("init.lua", false, '\u{e620}'),
    ("ps1.bash", false, '\u{f489}'),
    ("app.py", false, '\u{e606}'),
    ("index.ts", false, '\u{e628}'),
    ("main.go", false, '\u{e65e}'),
    ("main.c", false, '\u{e61e}'),
    ("index.html", false, '\u{f13b}'),
    ("style.css", false, '\u{e749}'),
    ("package.json", false, '\u{e71e}'),
    ("config.yaml", false, '\u{e8eb}'),
    ("README.md", false, '\u{f00ba}'),
    ("Makefile", false, '\u{e673}'),
    ("Dockerfile", false, '\u{e650}'),
    (".gitignore", false, '\u{f02a2}'),
    ("LICENSE", false, '\u{f02d}'),
    ("photo.png", false, '\u{f1c5}'),
    ("archive.zip", false, '\u{f410}'),
    ("notes.txt", false, '\u{f15c}'),
];

/// One name's cell in the listing: its icon, a gap, the name.
const COLUMN: usize = 20;

/// The spaces between an icon and its name, as `ls` will print them, so how
/// an icon sits against its name here is how it sits in a real listing.
///
/// One for a Nerd Font glyph, which is a cell wide. Two for Material:
/// wezterm.lua draws that font at 1.2 times the text's size, where an icon
/// covers its own cell and most of the next.
const NERD_GAP: usize = 1;
const MATERIAL_GAP: usize = 2;

/// `LISTING` down the page in as many columns as `cols` holds, each name
/// with the icon `icon_of` gives it and `gap` spaces after the icon.
fn listing(icon_of: impl Fn(&str, bool, char) -> char, gap: usize, cols: usize) -> Vec<String> {
    let gap = " ".repeat(gap);
    let across = (cols / COLUMN).clamp(1, 3);
    let down = LISTING.len().div_ceil(across);
    (0..down)
        .map(|row| {
            let mut line = String::from("  ");
            for column in 0..across {
                let Some((name, dir, eza)) = LISTING.get(column * down + row) else {
                    break;
                };
                let icon = icon_of(name, *dir, *eza);
                let slash = if *dir { "/" } else { "" };
                let cell = format!("{icon}{gap}{name}{slash}");
                // Never cut: the last column is the only one that could be,
                // and `across` was chosen so that it fits.
                let pad = COLUMN.saturating_sub(width::width(&cell));
                if *dir {
                    line.push_str(&format!("{icon}{gap}\x1b[1m{name}{slash}\x1b[0m"));
                } else {
                    line.push_str(&cell);
                }
                if column + 1 < across {
                    line.push_str(&" ".repeat(pad));
                }
            }
            line
        })
        .collect()
}

/// Every icon in the set, in name order. Two spaces after each: wezterm draws
/// a square glyph across the cell beside it only when that cell is a space,
/// and a sheet of them touching would show the shrunken form nobody will see
/// in a listing.
fn sheet(material: &Material, cols: usize) -> Vec<String> {
    const CELLS: usize = 3;
    let across = (cols / CELLS).max(1);
    material
        .icons
        .chunks(across)
        .map(|chunk| {
            let mut line = String::from("  ");
            for (_, c) in chunk {
                line.push(*c);
                line.push_str("  ");
            }
            line.trim_end().to_string()
        })
        .collect()
}

fn dim(text: &str) -> String {
    format!("\x1b[38;5;244m{text}\x1b[0m")
}

fn bold(text: &str) -> String {
    format!("\x1b[1m{text}\x1b[0m")
}

impl Screen for IconScreen {
    fn title(&self) -> String {
        "  icons — what draws a file's icon".into()
    }

    fn rows(&mut self) -> Vec<Row> {
        let now = now(&self.conf);
        SETS.iter()
            .map(|set| {
                let row = match (set, &self.material) {
                    (Set::Nerd, _) => {
                        Row::leaf(format!("{:<11}", "nerd font")).detail("one color · the default")
                    }
                    (Set::Material, Ok(m)) => Row::leaf(format!("{:<11}", "material"))
                        .detail(format!("color · {} icons", m.icons.len())),
                    (Set::Material, Err(_)) => {
                        Row::leaf(format!("{:<11}", "material")).detail("not installed")
                    }
                };
                if *set == now {
                    row.badged("now", 114)
                } else {
                    row
                }
            })
            .collect()
    }

    fn footer(&self) -> String {
        if !self.status.is_empty() {
            return self.status.clone();
        }
        "j/k move · enter set · l into the preview".into()
    }

    /// What the last key said belongs to the row it was pressed on.
    fn focus(&mut self, _sel: usize) {
        self.status.clear();
    }

    fn pane(&mut self, sel: usize, cols: u16, _rows: u16) -> Pane {
        let mut pane = Pane::default();
        let Some(set) = SETS.get(sel) else {
            return pane;
        };
        let wide = (cols as usize).saturating_sub(4);
        let mut say = |line: String| pane.lines.push(line);
        match (set, &self.material) {
            (Set::Nerd, _) => {
                say(bold("Nerd Font"));
                say(dim(
                    "The default: what `ls` and nvim draw when the file says",
                ));
                say(dim("nothing. One color: a glyph takes the color of the"));
                say(dim(
                    "name beside it. `ls` (eza) and nvim each carry their own",
                ));
                say(dim("list of which file gets which glyph."));
                say(String::new());
                for line in listing(|_, _, eza| eza, NERD_GAP, wide) {
                    say(line);
                }
                say(String::new());
                say(dim(
                    "  Drawn from the lane's own font when it is a Nerd Font, and",
                ));
                say(dim(
                    "  from the Symbols Nerd Font wezterm bundles when it is not.",
                ));
                say(String::new());
                say(dim(
                    "  Not part of a set: the prompt's markers and these tabs are",
                ));
                say(dim(
                    "  emoji (\u{1f30d} \u{1f512} \u{2705}), set by PS1_ICON_* in ~/.bash_local.",
                ));
            }
            (Set::Material, Err(why)) => {
                say(bold("Material Icon Theme"));
                say(String::new());
                say(format!("  {why}."));
                say(String::new());
                say(dim(
                    "  The font and its table ship in roest1/fonts, which the",
                ));
                say(dim(
                    "  dotfiles repo links there: update the submodule, then",
                ));
                say(dim("  `make link`."));
            }
            (Set::Material, Ok(material)) => {
                say(bold(
                    format!("Material Icon Theme {}", material.version).trim_end(),
                ));
                say(dim(&format!(
                    "{} icons in color, the set VS Code's extension of that name",
                    material.icons.len()
                )));
                say(dim(
                    "draws. Which file gets which icon is upstream's own list.",
                ));
                say(String::new());
                if !self.wezterm {
                    say("  This terminal is not WezTerm. These are private-use".into());
                    say("  characters: without the font they draw as empty boxes.".into());
                    say(String::new());
                }
                for line in listing(
                    |name, dir, _| material.icon_for(name, dir).unwrap_or(' '),
                    MATERIAL_GAP,
                    wide,
                ) {
                    say(line);
                }
                say(String::new());
                say(dim(
                    "  enter sets it. nvim takes it at once, in oil and telescope;",
                ));
                say(dim(
                    "  `ls` in the next shell, or in this one after `reload`.",
                ));
                say(String::new());
                say(dim(
                    "  In `ls`, a file or folder the set has no icon for keeps its",
                ));
                say(dim(
                    "  Nerd Font glyph: eza cannot be told what everything else",
                ));
                say(dim("  gets. In oil every folder gets the one folder icon."));
                say(String::new());
                say(dim("  Every icon, in name order (l, then j/k to scroll):"));
                say(String::new());
                for line in sheet(material, wide) {
                    say(line);
                }
            }
        }
        pane
    }

    fn on_action(&mut self, action: Action, sel: usize) -> Flow {
        let (Action::Activate, Some(set)) = (action, SETS.get(sel)) else {
            return Flow::Continue;
        };
        match self.pick(*set) {
            Err(why) => self.status = why,
            Ok((value, said)) => match conf::update(&[(KEY, value)]) {
                Ok(conf) => {
                    self.conf = conf;
                    self.status = said;
                }
                // Not saved, so not shown as if it were.
                Err(e) => self.status = format!("could not write: {e}"),
            },
        }
        Flow::Dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conf::merged;

    const TABLE: &str = "# Material Icon Theme 5.40.0: which code point is which icon, and which\n\
        # icon a file gets.\n\
        default\tfile\tfile\n\
        default\tfolder\tfolder\n\
        default\troot\tfolder-root\n\
        ext\td.ts\ttypescript-def\n\
        ext\trs\trust\n\
        ext\ttoml\ttoml\n\
        ext\tts\ttypescript\n\
        file\t.gitignore\tgit\n\
        file\treadme.md\treadme\n\
        folder\tsrc\tfolder-src\n\
        icon\tfile\t100000\n\
        icon\tfolder\t100001\n\
        icon\tfolder-src\t100002\n\
        icon\tgit\t100003\n\
        icon\treadme\t100004\n\
        icon\trust\t100005\n\
        icon\ttoml\t100006\n\
        icon\ttypescript\t100007\n\
        icon\ttypescript-def\t100008\n\
        icon\tnot-hex\tzz\n\
        lang\trust\trust\n\
        a line that is not a row\n";

    fn material() -> Material {
        Material::parse(TABLE).expect("a table")
    }

    fn at(n: u32) -> Option<char> {
        char::from_u32(0x100000 + n)
    }

    #[test]
    fn a_name_gets_the_icon_upstream_gives_it() {
        let m = material();
        assert_eq!(m.version, "5.40.0");
        assert_eq!(
            m.icons.len(),
            9,
            "the row that is not a code point is no icon"
        );
        assert_eq!(m.icon_for("main.rs", false), at(5));
        assert_eq!(m.icon_for("Cargo.toml", false), at(6), "by its extension");
        assert_eq!(
            m.icon_for("README.md", false),
            at(4),
            "a whole name, in any case"
        );
        assert_eq!(m.icon_for(".gitignore", false), at(3));
        assert_eq!(
            m.icon_for("index.d.ts", false),
            at(8),
            "the longest extension"
        );
        assert_eq!(m.icon_for("index.ts", false), at(7));
        assert_eq!(m.icon_for("notes", false), at(0), "nothing of its own");
        assert_eq!(m.icon_for("a.unknown", false), at(0));
        assert_eq!(m.icon_for("src", true), at(2));
        assert_eq!(m.icon_for("Src", true), at(2));
        assert_eq!(m.icon_for("elsewhere", true), at(1));
        assert_eq!(
            m.icon_for("src", false),
            at(0),
            "a file named like a folder"
        );
    }

    #[test]
    fn a_file_with_no_icons_is_no_table() {
        assert!(Material::parse("").is_none());
        assert!(Material::parse("# Material Icon Theme 5.40.0\next\trs\trust\n").is_none());
    }

    #[test]
    fn the_file_saying_nothing_is_the_default() {
        for (file, set) in [
            ("", Set::Nerd),
            ("icons = material\n", Set::Material),
            ("icons = nerd\n", Set::Nerd),
            ("icons = Material\n", Set::Nerd),
            ("icons = something-later\n", Set::Nerd),
        ] {
            assert_eq!(now(&merged(file, "")), set, "{file:?}");
        }
        assert_eq!(summary(&merged("icons = material\n", "")), "material");
    }

    #[test]
    fn a_pick_is_one_key_and_the_default_is_no_key() {
        let s = IconScreen::with(merged("", ""), Ok(material()), true);
        assert_eq!(s.pick(Set::Material).unwrap().0, Some("material"));
        assert_eq!(s.pick(Set::Nerd).unwrap().0, None);
    }

    /// Its characters are boxes without the font, so it is not set blind.
    #[test]
    fn material_is_not_picked_where_its_font_is_not() {
        let s = IconScreen::with(merged("", ""), Err("the font is not there".into()), true);
        assert!(s.pick(Set::Material).is_err());
        assert!(s.pick(Set::Nerd).is_ok(), "the default is always there");
    }

    #[test]
    fn nothing_drawn_is_wider_than_it_was_asked_to_be() {
        let m = material();
        for cols in [10, 20, 39, 40, 61, 100] {
            let lines = listing(|_, _, eza| eza, NERD_GAP, cols)
                .into_iter()
                .chain(listing(
                    |n, d, _| m.icon_for(n, d).unwrap_or(' '),
                    MATERIAL_GAP,
                    cols,
                ))
                .chain(sheet(&m, cols));
            for line in lines {
                // One column of the listing is as wide as its longest name
                // however narrow the pane; the pane cuts that itself.
                let room = cols.max(COLUMN) + 2;
                assert!(width::width(&line) <= room, "{cols}: {line:?}");
            }
        }
    }

    #[test]
    fn every_name_in_the_listing_is_drawn_once() {
        for cols in [10, 40, 61] {
            let text = listing(|_, _, eza| eza, NERD_GAP, cols).join("\n");
            for (name, _, _) in LISTING {
                assert_eq!(text.matches(name).count(), 1, "{cols}: {name}");
            }
        }
    }
}
