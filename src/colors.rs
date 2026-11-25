//! The colors screen, the Themes tab — the terminal's palette, and nvim's two
//! lanes.
//!
//! The same tree as the fonts: a lane, and under it what the lane can be set
//! to. Three lanes and not four, because `claude` has no colors of its own: a
//! Claude pane is a terminal pane, and wezterm colors a window.
//!
//! Nothing here knows what a theme looks like. A palette is read out of the
//! file wezterm reads, and a colorscheme is asked of nvim, so the preview is
//! drawn from the colors the lane will really get. See `schemes`.
//!
//! A pick is live: wezterm repaints when the file changes, and so does every
//! running nvim. The preview is for choosing; the terminal is the proof.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use tui_core::{nav::Action, width, Flow, Pane, Row, Screen, Tick};

use crate::conf::{self, Conf};
use crate::lanes::{self, COLOR_LANES, DEFAULT_PALETTE, DEFAULT_SCHEME};
use crate::schemes::{self, Palette, Rgb, Scheme};

const SHELL: usize = 0;
const UI: usize = 1;
const EDITOR: usize = 2;

/// How often to look for nvim's answer while it is on its way.
const ASKING: Duration = Duration::from_millis(30);

/// What `enter` on the shell lane's opacity row steps through, in percent.
const OPACITY: [u8; 5] = [100, 95, 90, 85, 80];

/// A key and what to set it to; `None` takes the key out of the file.
type Change = (String, Option<String>);

/// What the file says, with every gap filled the way the readers fill it.
#[derive(Debug, PartialEq)]
struct Now {
    palette: String,
    /// Percent. 100 is opaque, which is also what an unreadable value means.
    opacity: u8,
    ui: String,
    ui_transparent: bool,
    /// The editor lane has no colorscheme of its own and takes the ui lane's.
    editor_follows: bool,
    editor: String,
    editor_transparent: bool,
}

/// The same rules as wezterm.lua and nvim's external/theme.lua, which is what
/// makes the screen show what the readers will do and not what was typed:
/// opacity is a number from 0.3 up to 1, `transparent` is true unless it says
/// `false`, and an editor key left out follows the ui lane.
fn now(conf: &Conf) -> Now {
    let get = |key: &str| conf.get(key).map(String::as_str);
    let opacity = get("shell.opacity")
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (0.3..1.0).contains(v))
        .map_or(100, |v| (v * 100.0).round() as u8);
    let ui = get("nvim.ui.colors").unwrap_or(DEFAULT_SCHEME).to_string();
    let ui_transparent = get("nvim.ui.transparent") != Some("false");
    Now {
        palette: get("shell.colors").unwrap_or(DEFAULT_PALETTE).to_string(),
        opacity,
        editor_follows: get("nvim.editor.colors").is_none(),
        editor: get("nvim.editor.colors").unwrap_or(&ui).to_string(),
        editor_transparent: get("nvim.editor.transparent").map_or(ui_transparent, |v| v != "false"),
        ui,
        ui_transparent,
    }
}

impl Now {
    fn scheme(&self, lane: usize) -> &str {
        if lane == UI {
            &self.ui
        } else {
            &self.editor
        }
    }

    fn transparent(&self, lane: usize) -> bool {
        if lane == UI {
            self.ui_transparent
        } else {
            self.editor_transparent
        }
    }

    /// The lane's row, in a few words.
    fn summary(&self, lane: usize) -> String {
        // One word each: this is the lane's row, in a narrow column.
        let background = |transparent: bool| {
            if transparent {
                "transparent"
            } else {
                "opaque"
            }
        };
        match lane {
            SHELL if self.opacity < 100 => format!("{} · {}% opaque", self.palette, self.opacity),
            SHELL => self.palette.clone(),
            EDITOR if self.editor_follows && self.editor_transparent == self.ui_transparent => {
                "follows nvim.ui".into()
            }
            _ => format!(
                "{} · {}",
                self.scheme(lane),
                background(self.transparent(lane))
            ),
        }
    }
}

/// Each color lane and what it is set to, for `theme show`.
pub fn summary(conf: &Conf) -> Vec<(&'static str, String)> {
    let now = now(conf);
    (0..COLOR_LANES.len())
        .map(|lane| (COLOR_LANES[lane], now.summary(lane)))
        .collect()
}

fn covers(lane: usize) -> &'static str {
    match lane {
        SHELL => "the terminal: its background and tab bar, under every pane",
        UI => "nvim's chrome — oil, telescope, statusline, gutter",
        _ => "windows showing a file you write in nvim",
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Entry {
    /// A blank row between lanes.
    Gap,
    /// A lane, by its place in `COLOR_LANES`.
    Lane(usize),
    /// One of wezterm's themes, under `shell`.
    Palette(usize),
    /// "same as nvim.ui", the first row under `nvim.editor`.
    Follow,
    /// One of nvim's colorschemes under a lane: the lane, then the scheme.
    Scheme(usize, usize),
    /// Where a list would be: nvim has not answered, or has nothing to list.
    /// Explains itself in the pane and does nothing.
    Nothing(usize),
    /// The last row under a lane, its background: how opaque the terminal
    /// is, or whether an nvim lane paints one at all.
    Background(usize),
}

pub struct ColorScreen {
    conf: Conf,
    palettes: Vec<Palette>,
    /// nvim's colorschemes, or why there are none. `None` while it is asked.
    schemes: Option<Result<Vec<Scheme>, String>>,
    arriving: Option<Receiver<Result<Vec<Scheme>, String>>>,
    open: Option<usize>,
    /// The row to put the cursor on after the tree changes shape.
    land: usize,
    status: String,
}

impl Default for ColorScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorScreen {
    /// Opens at once. nvim is asked on a thread of its own: it answers in a
    /// tenth of a second, which is still a frozen first frame if waited for.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(schemes::ask_nvim());
        });
        let mut me = Self::with(conf::load(), schemes::palettes(), None);
        me.arriving = Some(rx);
        me
    }

    fn with(
        conf: Conf,
        palettes: Vec<Palette>,
        schemes: Option<Result<Vec<Scheme>, String>>,
    ) -> Self {
        Self {
            conf,
            palettes,
            schemes,
            arriving: None,
            open: None,
            land: 0,
            status: String::new(),
        }
    }

    fn scheme_list(&self) -> &[Scheme] {
        match &self.schemes {
            Some(Ok(list)) => list,
            _ => &[],
        }
    }

    /// Which rows are which, in display order. `rows`, `pane` and
    /// `on_action` all read this, so they cannot disagree about a row.
    fn layout(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        for lane in 0..COLOR_LANES.len() {
            if lane > 0 {
                out.push(Entry::Gap);
            }
            out.push(Entry::Lane(lane));
            if self.open != Some(lane) {
                continue;
            }
            if lane == SHELL {
                if self.palettes.is_empty() {
                    out.push(Entry::Nothing(lane));
                }
                out.extend((0..self.palettes.len()).map(Entry::Palette));
            } else {
                if lane == EDITOR {
                    out.push(Entry::Follow);
                }
                let schemes = self.scheme_list();
                if schemes.is_empty() {
                    out.push(Entry::Nothing(lane));
                }
                out.extend((0..schemes.len()).map(|s| Entry::Scheme(lane, s)));
            }
            out.push(Entry::Background(lane));
        }
        out
    }

    /// Open `open` (or nothing) and put the cursor on `lane`'s own row.
    /// `Flow::Retarget`, because every row below a lane moves when it opens
    /// or closes and `Dirty` would keep the cursor's number.
    fn show(&mut self, open: Option<usize>, lane: usize) -> Flow {
        self.open = open;
        self.land = self
            .layout()
            .iter()
            .position(|e| *e == Entry::Lane(lane))
            .unwrap_or(0);
        Flow::Retarget
    }

    /// What `enter` on `entry` writes, and what to say once it has. `Err` is
    /// a row that will not be set, with the reason; `Ok(None)` is a row with
    /// nothing to set.
    fn pick(&self, entry: Entry) -> Result<Option<(Vec<Change>, String)>, String> {
        let now = now(&self.conf);
        let set = |key: String, value: &str| (key, Some(value.to_string()));
        Ok(Some(match entry {
            Entry::Gap | Entry::Lane(_) | Entry::Nothing(_) => return Ok(None),
            Entry::Palette(p) => {
                let name = &self.palettes[p].name;
                (
                    vec![set("shell.colors".into(), name)],
                    format!("shell → {name}"),
                )
            }
            Entry::Follow => (
                vec![
                    ("nvim.editor.colors".into(), None),
                    ("nvim.editor.transparent".into(), None),
                ],
                "nvim.editor → follows nvim.ui".into(),
            ),
            Entry::Scheme(lane, s) => {
                let (lane_name, name) = (COLOR_LANES[lane], &self.scheme_list()[s].name);
                let mut changes = vec![set(format!("{lane_name}.colors"), name)];
                if lanes::opaque_only(name) {
                    changes.push(set(format!("{lane_name}.transparent"), "false"));
                }
                (changes, format!("{lane_name} → {name}"))
            }
            Entry::Background(SHELL) => {
                let next = OPACITY
                    .iter()
                    .copied()
                    .find(|step| *step < now.opacity)
                    .unwrap_or(100);
                // 100 is the key left out, so "off" stays wezterm's default.
                let value = (next < 100).then(|| format!("{:.2}", f32::from(next) / 100.0));
                (
                    vec![("shell.opacity".into(), value)],
                    format!("shell → {next}% opaque"),
                )
            }
            Entry::Background(lane) => {
                let (lane_name, scheme) = (COLOR_LANES[lane], now.scheme(lane));
                let to = !now.transparent(lane);
                if to && lanes::opaque_only(scheme) {
                    return Err(format!(
                        "{scheme} is not offered transparent — it reads badly over the terminal's background"
                    ));
                }
                (
                    vec![set(
                        format!("{lane_name}.transparent"),
                        if to { "true" } else { "false" },
                    )],
                    format!(
                        "{lane_name} → {}",
                        if to {
                            "transparent"
                        } else {
                            "its own background"
                        }
                    ),
                )
            }
        }))
    }

    /// The lane a row belongs to.
    fn lane_of(entry: Entry) -> Option<usize> {
        match entry {
            Entry::Gap => None,
            Entry::Palette(_) => Some(SHELL),
            Entry::Follow => Some(EDITOR),
            Entry::Lane(l) | Entry::Scheme(l, _) | Entry::Nothing(l) | Entry::Background(l) => {
                Some(l)
            }
        }
    }
}

// ─── drawing ────────────────────────────────────────────────────────────────

/// Whether the terminal takes 24-bit color. A preview drawn in 256 colors
/// would be a picture of some other theme.
fn truecolor() -> bool {
    matches!(
        std::env::var("COLORTERM").as_deref(),
        Ok("truecolor" | "24bit")
    )
}

fn fg(c: Rgb) -> String {
    format!("\x1b[38;2;{};{};{}m", c[0], c[1], c[2])
}

fn bg(c: Rgb) -> String {
    format!("\x1b[48;2;{};{};{}m", c[0], c[1], c[2])
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// A stretch of a preview line: its text, its color, and a background of its
/// own where it has one.
type Run<'a> = (&'a str, Option<Rgb>, Option<Rgb>);

/// One line of a preview, exactly `cols` cells wide: each run in its colors
/// over `back`, then `back` out to the edge. `None` is the terminal's own
/// color, which for a background is exactly what transparent means.
///
/// Cut here, to the cell, when the pane is narrower than the text. Left to
/// the pane's own truncation the block would lose its right edge raggedly,
/// one line at a time.
fn paint(runs: &[Run], back: Option<Rgb>, cols: usize) -> String {
    let mut line = String::from("  ");
    let mut used = 0;
    for (text, color, own) in runs {
        line.push_str("\x1b[0m");
        if let Some(c) = own.or(back) {
            line.push_str(&bg(c));
        }
        if let Some(c) = color {
            line.push_str(&fg(*c));
        }
        for ch in text.chars() {
            let cells = width::char_width(ch);
            if used + cells > cols {
                break;
            }
            line.push(ch);
            used += cells;
        }
    }
    line.push_str("\x1b[0m");
    if let Some(c) = back {
        line.push_str(&bg(c));
    }
    line.push_str(&" ".repeat(cols.saturating_sub(used)));
    line.push_str("\x1b[0m");
    line
}

/// The terminal a palette makes: its tab bar, a prompt, a selection.
fn palette_preview(p: &Palette, cols: usize) -> Vec<String> {
    let on = |text, color| (text, Some(color), None);
    vec![
        paint(
            &[
                (" ", None, Some(p.surface)),
                ("▎", Some(p.accent), Some(p.tab_active)),
                ("1 shell  ", Some(p.text), Some(p.tab_active)),
                ("  2 nvim  ", Some(p.muted), Some(p.surface)),
                ("  3 claude  ", Some(p.text), Some(p.overlay)),
                ("  +", Some(p.subtle), Some(p.surface)),
            ],
            Some(p.surface),
            cols,
        ),
        paint(&[], Some(p.base), cols),
        paint(
            &[
                on(" ~/dotfiles ", p.text),
                on("main ", p.subtle),
                on("❯ ls", p.text),
            ],
            Some(p.base),
            cols,
        ),
        paint(
            &[on(" deps.conf  Makefile  tui/", p.text)],
            Some(p.base),
            cols,
        ),
        paint(
            &[
                on(" ", p.text),
                (" a selection ", Some(p.text), Some(p.highlight_med)),
                on("  and text that is not the focus", p.muted),
            ],
            Some(p.base),
            cols,
        ),
        paint(&[], Some(p.base), cols),
    ]
}

/// What a file looks like in a colorscheme, by highlight group.
const CODE: [&[(&str, &str)]; 6] = [
    &[("-- what each lane is set to", "Comment")],
    &[
        ("pub fn ", "Keyword"),
        ("read", "Function"),
        ("() -> ", "Delimiter"),
        ("Vec", "Type"),
        ("<", "Delimiter"),
        ("String", "Type"),
        ("> {", "Delimiter"),
    ],
    &[
        ("    let ", "Keyword"),
        ("name", "Identifier"),
        (" = ", "Operator"),
        ("\"0xProto Nerd Font\"", "String"),
        (";", "Delimiter"),
    ],
    &[
        ("    let ", "Keyword"),
        ("size", "Identifier"),
        (" = ", "Operator"),
        ("14", "Number"),
        (";", "Delimiter"),
    ],
    &[
        ("    ", "Normal"),
        ("lanes", "Function"),
        ("(", "Delimiter"),
        ("name", "Identifier"),
        (", ", "Delimiter"),
        ("size", "Identifier"),
        (")", "Delimiter"),
    ],
    &[("}", "Delimiter")],
];

/// The line of `CODE` the cursor is drawn on.
const CURSOR_LINE: usize = 2;

/// A window in a colorscheme: line numbers, a cursor line, a statusline.
/// Transparent leaves the background out, so the terminal's shows through
/// here the way it will in nvim.
fn scheme_preview(s: &Scheme, transparent: bool, cols: usize) -> Vec<String> {
    let back = if transparent { None } else { s.bg("Normal") };
    let ink = |group: &str| s.fg(group).or(s.fg("Normal"));
    let mut out: Vec<String> = CODE
        .iter()
        .enumerate()
        .map(|(n, tokens)| {
            let cursor = n == CURSOR_LINE;
            let row = cursor.then(|| s.bg("CursorLine")).flatten();
            let number = format!(" {} ", n + 1);
            let mut runs: Vec<Run> = vec![(
                &number,
                ink(if cursor { "CursorLineNr" } else { "LineNr" }),
                None,
            )];
            runs.extend(tokens.iter().map(|(text, group)| (*text, ink(group), row)));
            // The cursor line's own background runs to the edge.
            paint(&runs, row.or(back), cols)
        })
        .collect();
    out.push(paint(
        &[(
            " NORMAL  lanes.rs  3:9 ",
            s.fg("StatusLine").or(s.fg("Normal")),
            None,
        )],
        s.bg("StatusLine").or(back),
        cols,
    ));
    out
}

fn dim(text: &str) -> String {
    format!("\x1b[38;5;244m{text}\x1b[0m")
}

impl Screen for ColorScreen {
    fn title(&self) -> String {
        "  themes — the terminal, and nvim's two lanes".into()
    }

    fn initial_sel(&self) -> usize {
        self.land
    }

    fn tick(&mut self) -> Tick {
        let Some(rx) = &self.arriving else {
            return Tick::Idle;
        };
        match rx.try_recv() {
            Ok(answer) => {
                self.schemes = Some(answer);
                self.arriving = None;
                Tick::Changed
            }
            Err(TryRecvError::Empty) => Tick::Wake(ASKING),
            Err(TryRecvError::Disconnected) => {
                self.schemes = Some(Err("nvim could not be asked".into()));
                self.arriving = None;
                Tick::Changed
            }
        }
    }

    fn rows(&mut self) -> Vec<Row> {
        let now = now(&self.conf);
        let under = |row: Row| {
            let mut row = row;
            row.depth = 1;
            row
        };
        let marked = |row: Row, is_now: bool| {
            if is_now {
                row.badged("now", 114)
            } else {
                row
            }
        };
        self.layout()
            .into_iter()
            .map(|entry| match entry {
                Entry::Gap => Row::spacer(),
                Entry::Lane(lane) => {
                    let mut row =
                        Row::leaf(format!("{:<11}", COLOR_LANES[lane])).detail(now.summary(lane));
                    row.group = true;
                    row.expanded = self.open == Some(lane);
                    row
                }
                Entry::Palette(p) => {
                    let name = &self.palettes[p].name;
                    marked(under(Row::leaf(name)), *name == now.palette)
                }
                Entry::Follow => marked(under(Row::leaf("same as nvim.ui")), now.editor_follows),
                Entry::Scheme(lane, s) => {
                    let name = &self.scheme_list()[s].name;
                    let follows = lane == EDITOR && now.editor_follows;
                    marked(under(Row::leaf(name)), !follows && name == now.scheme(lane))
                }
                Entry::Nothing(lane) => under(Row::leaf(match (lane, &self.schemes) {
                    (SHELL, _) => "no themes found",
                    (_, None) => "asking nvim\u{2026}",
                    (_, Some(_)) => "no colorschemes",
                })),
                Entry::Background(SHELL) => {
                    under(Row::leaf("opacity").detail(format!("{}%", now.opacity)))
                }
                Entry::Background(lane) => {
                    under(Row::leaf("background").detail(if now.transparent(lane) {
                        "transparent"
                    } else {
                        "its own"
                    }))
                }
            })
            .collect()
    }

    fn footer(&self) -> String {
        if !self.status.is_empty() {
            return self.status.clone();
        }
        "j/k move · l open · enter set · gg/G".into()
    }

    /// What the last key said belongs to the row it was pressed on.
    fn focus(&mut self, _sel: usize) {
        self.status.clear();
    }

    fn pane(&mut self, sel: usize, cols: u16, _rows: u16) -> Pane {
        let mut pane = Pane::default();
        let Some(entry) = self.layout().get(sel).copied() else {
            return pane;
        };
        let now = now(&self.conf);
        let wide = (cols as usize).saturating_sub(4);
        let mut say = |line: String| pane.lines.push(line);
        match entry {
            Entry::Gap => {}
            Entry::Lane(lane) => {
                say(format!("\x1b[1m{}\x1b[0m", COLOR_LANES[lane]));
                say(dim(covers(lane)));
                say(String::new());
                say(format!("  now: {}", now.summary(lane)));
                say(String::new());
                say(dim(if lane == SHELL {
                    "  l or → to list the themes"
                } else {
                    "  l or → to list nvim's colorschemes"
                }));
            }
            Entry::Palette(p) => {
                let palette = &self.palettes[p];
                say(format!("\x1b[1m{}\x1b[0m", palette.name));
                say(String::new());
                if truecolor() {
                    for line in palette_preview(palette, wide) {
                        say(line);
                    }
                    say(String::new());
                }
                for (name, color) in palette.named() {
                    let swatch = if truecolor() {
                        format!("{}    \x1b[0m ", bg(color))
                    } else {
                        String::new()
                    };
                    say(format!("  {swatch}{name:<14}{}", hex(color)));
                }
                say(String::new());
                say(dim(
                    "  The ANSI 16 are not part of a theme: gh, git and the",
                ));
                say(dim(
                    "  prompt keep the hues they have, over a new background.",
                ));
            }
            Entry::Follow => {
                say("\x1b[1msame as nvim.ui\x1b[0m".into());
                say(String::new());
                say(format!("  nvim.ui is {}", now.summary(UI)));
                say(String::new());
                say(dim(
                    "  One colorscheme for the whole of nvim. Setting this takes",
                ));
                say(dim(
                    "  the editor lane's own picks out of the file, so a later",
                ));
                say(dim("  change to nvim.ui moves both."));
            }
            Entry::Scheme(lane, s) => {
                let scheme = &self.scheme_list()[s];
                let lane_name = COLOR_LANES[lane];
                let opaque_only = lanes::opaque_only(&scheme.name);
                let transparent = now.transparent(lane) && !opaque_only;
                say(format!("\x1b[1m{}\x1b[0m", scheme.name));
                say(String::new());
                if truecolor() {
                    for line in scheme_preview(scheme, transparent, wide) {
                        say(line);
                    }
                } else {
                    say(dim("  (the preview needs a 24-bit color terminal)"));
                }
                say(String::new());
                say(if opaque_only {
                    "  set with its own background: over the terminal's it reads badly".into()
                } else if transparent {
                    format!("  {lane_name} is transparent: the terminal's background shows")
                } else {
                    match scheme.bg("Normal") {
                        Some(c) => format!("  {lane_name} paints its own background, {}", hex(c)),
                        None => format!("  {lane_name} paints its own background"),
                    }
                });
                say(dim("  enter sets it, in every nvim that is running"));
            }
            Entry::Nothing(lane) => match (lane, &self.schemes) {
                (SHELL, _) => {
                    say("\x1b[1mno themes found\x1b[0m".into());
                    say(String::new());
                    say("  ~/.config/wezterm/themes has no theme in it.".into());
                    say(String::new());
                    say(dim(
                        "  It is linked from the dotfiles repo: run `make link`.",
                    ));
                }
                (_, None) => say(dim("  asking nvim which colorschemes it has\u{2026}")),
                (_, Some(Err(why))) => {
                    say("\x1b[1mno colorschemes\x1b[0m".into());
                    say(String::new());
                    say(format!("  {why}."));
                }
                (_, Some(Ok(_))) => {}
            },
            Entry::Background(SHELL) => {
                say("\x1b[1mopacity\x1b[0m".into());
                say(String::new());
                say(format!("  now: {}%", now.opacity));
                say(String::new());
                say(dim(
                    "  How much of the desktop shows through the terminal's",
                ));
                say(dim(
                    "  background. Text stays opaque. enter steps down to 80%",
                ));
                say(dim("  and back round to 100%."));
            }
            Entry::Background(lane) => {
                let (lane_name, scheme) = (COLOR_LANES[lane], now.scheme(lane));
                say("\x1b[1mbackground\x1b[0m".into());
                say(String::new());
                say(format!(
                    "  now: {}",
                    if now.transparent(lane) {
                        "transparent — the terminal's background shows"
                    } else {
                        "its own — the colorscheme paints it"
                    }
                ));
                say(String::new());
                if lanes::opaque_only(scheme) {
                    say(dim(&format!("  {scheme} is not offered transparent.")));
                } else {
                    say(dim(&format!("  enter switches it for {lane_name}.")));
                }
            }
        }
        pane
    }

    fn on_action(&mut self, action: Action, sel: usize) -> Flow {
        let Some(entry) = self.layout().get(sel).copied() else {
            return Flow::Continue;
        };
        self.status.clear();
        let lane = Self::lane_of(entry);
        match (action, entry) {
            // `l` opens a closed lane. On an open one it has nothing left to
            // open, and `Continue` is what lets it move on into the pane.
            (Action::Open, Entry::Lane(l)) if self.open != Some(l) => self.show(Some(l), l),
            (Action::Close, _) if lane.is_some() && self.open == lane => {
                self.show(None, lane.unwrap_or(0))
            }
            (Action::Activate, Entry::Lane(l)) => {
                let open = (self.open != Some(l)).then_some(l);
                self.show(open, l)
            }
            // Enter alone sets something. `l` only ever opens or moves right.
            (Action::Activate, _) => {
                match self.pick(entry) {
                    Ok(None) => return Flow::Continue,
                    Err(why) => self.status = why,
                    Ok(Some((changes, said))) => {
                        let changes: Vec<(&str, Option<&str>)> = changes
                            .iter()
                            .map(|(key, value)| (key.as_str(), value.as_deref()))
                            .collect();
                        match conf::update(&changes) {
                            Ok(conf) => {
                                self.conf = conf;
                                self.status = said;
                            }
                            // Not saved, so not shown as if it were.
                            Err(e) => self.status = format!("could not write: {e}"),
                        }
                    }
                }
                Flow::Dirty
            }
            _ => Flow::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conf::merged;
    use crate::schemes::parse_schemes;

    fn palette(name: &str) -> Palette {
        Palette::parse(
            name,
            "base = #282a36\ntab_active = #191a21\naccent = #bd93f9\nsurface = #21222c\n\
             overlay = #343746\nhighlight_med = #44475a\ntext = #f8f8f2\nmuted = #6272a4\n\
             subtle = #adb5cb\n",
        )
        .unwrap()
    }

    fn schemes() -> Vec<Scheme> {
        parse_schemes(
            "scheme\tdracula\tNormal=f8f8f2,282a36\tComment=6272a4,\tCursorLine=,44475a\n\
             scheme\tonedark\tNormal=abb2bf,282c34\n\
             scheme\trose-pine-moon\tNormal=e0def4,232136\n",
        )
    }

    fn screen(file: &str) -> ColorScreen {
        ColorScreen::with(
            merged(file, ""),
            vec![palette("dracula"), palette("rose-pine")],
            Some(Ok(schemes())),
        )
    }

    fn written(s: &ColorScreen, entry: Entry) -> Vec<(String, Option<String>)> {
        s.pick(entry).unwrap().unwrap().0
    }

    fn set(key: &str, value: &str) -> (String, Option<String>) {
        (key.to_string(), Some(value.to_string()))
    }

    /// The screen has to say what wezterm and nvim will DO with the file.
    #[test]
    fn the_file_is_read_the_way_the_readers_read_it() {
        let empty = now(&merged("", ""));
        assert_eq!(empty.palette, "rose-pine");
        assert_eq!(empty.opacity, 100);
        assert_eq!(
            (empty.ui.as_str(), empty.ui_transparent),
            ("rose-pine-moon", true)
        );
        assert!(empty.editor_follows);
        assert_eq!(
            (empty.editor.as_str(), empty.editor_transparent),
            ("rose-pine-moon", true)
        );

        // An editor key left out follows the ui lane, each on its own.
        let n = now(&merged(
            "nvim.ui.colors = dracula\nnvim.ui.transparent = false\n",
            "",
        ));
        assert_eq!(
            (n.editor.as_str(), n.editor_transparent),
            ("dracula", false)
        );
        let n = now(&merged(
            "nvim.ui.colors = dracula\nnvim.editor.transparent = false\n",
            "",
        ));
        assert!(n.editor_follows && n.ui_transparent && !n.editor_transparent);
        assert_eq!(n.summary(EDITOR), "dracula · opaque");

        // Only `false` is false, and only 0.3 up to 1 is an opacity.
        assert!(now(&merged("nvim.ui.transparent = no\n", "")).ui_transparent);
        for (value, pct) in [
            ("0.9", 90),
            ("0.30", 30),
            ("1", 100),
            ("0.29", 100),
            ("90", 100),
            ("x", 100),
        ] {
            let n = now(&merged(&format!("shell.opacity = {value}\n"), ""));
            assert_eq!(n.opacity, pct, "shell.opacity = {value}");
        }
    }

    #[test]
    fn a_pick_writes_its_own_keys_and_no_others() {
        let s = screen("");
        assert_eq!(
            written(&s, Entry::Palette(0)),
            [set("shell.colors", "dracula")]
        );
        assert_eq!(
            written(&s, Entry::Scheme(UI, 0)),
            [set("nvim.ui.colors", "dracula")]
        );
        assert_eq!(
            written(&s, Entry::Scheme(EDITOR, 2)),
            [set("nvim.editor.colors", "rose-pine-moon")]
        );
        assert_eq!(
            written(&s, Entry::Follow),
            [
                ("nvim.editor.colors".to_string(), None),
                ("nvim.editor.transparent".to_string(), None)
            ]
        );
        assert_eq!(
            written(&s, Entry::Background(UI)),
            [set("nvim.ui.transparent", "false")]
        );
        assert_eq!(
            written(&s, Entry::Background(EDITOR)),
            [set("nvim.editor.transparent", "false")]
        );
    }

    /// A row that is there to explain something sets nothing.
    #[test]
    fn a_row_with_nothing_to_set_writes_nothing() {
        let s = screen("");
        for entry in [Entry::Gap, Entry::Lane(SHELL), Entry::Nothing(UI)] {
            assert_eq!(s.pick(entry), Ok(None), "{entry:?}");
        }
    }

    /// "Tried and rejected" in dotfiles' plugin list. The readers allow it;
    /// the picker is where the taste lives.
    #[test]
    fn one_dark_is_never_put_over_the_terminals_background() {
        let s = screen("nvim.ui.transparent = true\n");
        assert_eq!(
            written(&s, Entry::Scheme(UI, 1)),
            [
                set("nvim.ui.colors", "onedark"),
                set("nvim.ui.transparent", "false")
            ]
        );
        let s = screen("nvim.ui.colors = onedark\nnvim.ui.transparent = false\n");
        assert!(
            s.pick(Entry::Background(UI)).is_err(),
            "not switched to transparent"
        );
        // Already transparent by hand: switching it off is still allowed.
        let s = screen("nvim.ui.colors = onedark\n");
        assert_eq!(
            written(&s, Entry::Background(UI)),
            [set("nvim.ui.transparent", "false")]
        );
    }

    #[test]
    fn opacity_steps_down_and_comes_back_round() {
        let step = |file: &str| written(&screen(file), Entry::Background(SHELL));
        assert_eq!(step(""), [set("shell.opacity", "0.95")]);
        assert_eq!(
            step("shell.opacity = 0.95\n"),
            [set("shell.opacity", "0.90")]
        );
        // A value set by hand goes to the next step below it.
        assert_eq!(
            step("shell.opacity = 0.87\n"),
            [set("shell.opacity", "0.85")]
        );
        // Back to opaque is the key taken out, not `1.00`.
        assert_eq!(
            step("shell.opacity = 0.80\n"),
            [("shell.opacity".to_string(), None)]
        );
        assert_eq!(
            step("shell.opacity = 0.5\n"),
            [("shell.opacity".to_string(), None)]
        );
    }

    #[test]
    fn opening_a_lane_lands_on_it_and_lists_what_it_can_be() {
        let mut s = screen("nvim.editor.colors = onedark\n");
        assert!(matches!(s.show(Some(EDITOR), EDITOR), Flow::Retarget));
        let layout = s.layout();
        assert_eq!(layout[s.land], Entry::Lane(EDITOR));
        assert_eq!(
            &layout[s.land + 1..],
            [
                Entry::Follow,
                Entry::Scheme(EDITOR, 0),
                Entry::Scheme(EDITOR, 1),
                Entry::Scheme(EDITOR, 2),
                Entry::Background(EDITOR)
            ]
        );
        let rows = s.rows();
        assert_eq!(rows.len(), layout.len(), "a row for every entry");
        let now_rows: Vec<&str> = rows
            .iter()
            .filter(|r| r.badges.iter().any(|(text, _)| text == "now"))
            .map(|r| r.label.as_str())
            .collect();
        assert_eq!(now_rows, ["onedark"], "one row is what the lane is set to");

        // Closing from a row under the lane lands back on the lane.
        let sel = s.land + 2;
        assert!(matches!(s.on_action(Action::Close, sel), Flow::Retarget));
        assert_eq!(s.open, None);
        assert_eq!(s.layout()[s.land], Entry::Lane(EDITOR));
    }

    /// Until nvim answers, and when it cannot, the lane still opens and says so.
    #[test]
    fn a_lane_with_no_list_says_why() {
        for (schemes, label) in [
            (None, "asking nvim\u{2026}"),
            (
                Some(Err("nvim is not installed".to_string())),
                "no colorschemes",
            ),
        ] {
            let mut s = ColorScreen::with(merged("", ""), vec![], schemes);
            s.show(Some(UI), UI);
            assert_eq!(
                &s.layout()[s.land + 1..s.land + 3],
                [Entry::Nothing(UI), Entry::Background(UI)]
            );
            assert_eq!(s.rows()[s.land + 1].label, label);
        }
        let mut s = ColorScreen::with(merged("", ""), vec![], Some(Ok(schemes())));
        s.show(Some(SHELL), SHELL);
        assert_eq!(s.rows()[1].label, "no themes found");
    }

    /// Measured in cells: the block has to end where the pane does.
    #[test]
    fn a_preview_line_is_as_wide_as_it_was_asked_to_be() {
        let schemes = schemes();
        // 40 cuts the longest line, 12 cuts nearly all of them, 80 cuts none.
        for cols in [12, 40, 80] {
            for line in palette_preview(&palette("dracula"), cols)
                .iter()
                .chain(&scheme_preview(&schemes[0], false, cols))
            {
                assert_eq!(width::width(line), cols + 2, "{cols}: {line:?}");
            }
        }
    }

    #[test]
    fn a_transparent_preview_paints_no_background_of_its_own() {
        let dracula = &schemes()[0];
        let opaque = scheme_preview(dracula, false, 40).join("\n");
        let clear = scheme_preview(dracula, true, 40).join("\n");
        assert!(opaque.contains(&bg([0x28, 0x2a, 0x36])));
        assert!(!clear.contains(&bg([0x28, 0x2a, 0x36])));
        // The cursor line is the scheme's either way.
        assert!(clear.contains(&bg([0x44, 0x47, 0x5a])));
        // A group the scheme did not set is drawn in its Normal.
        assert!(clear.contains(&fg([0xf8, 0xf8, 0xf2])));
    }
}
