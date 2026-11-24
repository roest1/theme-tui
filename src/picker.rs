//! The picker screen — the tree of lanes, and what each one is set to.
//!
//! The tree is the four lanes; opening one lists the installed families under
//! it with a fit verdict, and Enter sets it. That shape is not decoration: it
//! is the same tree `dots` needs, exercised on a screen small enough to get
//! right, so the model is proven before the bigger screen leans on it.
//!
//! Why a lane has a VERDICT rather than a score: a font is not good or bad on
//! its own, it is good or bad AT A JOB. The same family is the right pick for
//! `shell` and the wrong one for `claude`, and no single number says so.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tui_core::{nav::Action, Flow, Image, Pane, Row, Screen, Tick};

use crate::lanes::LANES;
use crate::{browse, fonts, lanes, render, theme};

/// The specimen's type size when a screen opens, in pixels.
pub(crate) const SPECIMEN_PX: f32 = 20.0;

/// How often to look for fonts that have finished loading while some are
/// still on their way.
const MEASURING: Duration = Duration::from_millis(30);

/// A font read off disk: what it measures, and the face to draw it in.
pub(crate) struct Loaded {
    pub metrics: fonts::Metrics,
    pub font: fontdue::Font,
    /// The names of the family's cuts.
    pub cuts: Vec<String>,
}

impl Loaded {
    /// Measure the file at `path` and load it for drawing. The one place both
    /// screens get a font from, so they cannot disagree about what one is.
    pub fn read(path: &str, cuts: Vec<String>) -> Option<Self> {
        let (data, metrics) = fonts::measure(path)?;
        Some(Self {
            metrics,
            font: render::load(&data)?,
            cuts,
        })
    }

    /// An installed family, from its upright cut.
    fn installed(family: &str) -> Option<Self> {
        let cuts: Vec<(String, String)> = fonts::cuts_of(family).into_iter().collect();
        let names = cuts.iter().map(|(style, _)| style.clone()).collect();
        Self::read(&fonts::upright(&cuts)?, names)
    }
}

#[derive(PartialEq, Clone, Copy)]
pub enum Preview {
    /// Every family in view, one row each, for comparing.
    Sheet,
    /// One family doing the lane's actual job, for committing.
    Specimen,
}

pub struct FontScreen {
    families: Vec<String>,
    /// Set on the way into the browse screen, which can install a family:
    /// the list is read again on the way back, so what was just fetched for a
    /// lane can be given to it without restarting `font`.
    reread: bool,
    state: Vec<(String, String)>,
    open: Option<usize>,
    /// The row to put the cursor on after the tree changes shape.
    land: usize,
    /// Every family read so far. `None` is one that could not be read; a
    /// family not in here at all is still on its way.
    cache: HashMap<String, Option<Loaded>>,
    /// Fonts still being read, off this thread.
    arriving: Option<Receiver<(String, Option<Loaded>)>>,
    status: String,
    preview: Preview,
    size: f32,
}

impl Default for FontScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl FontScreen {
    pub fn new() -> Self {
        let mut me = Self {
            families: fonts::families(),
            reread: false,
            state: lanes::read(),
            open: None,
            land: 0,
            cache: HashMap::new(),
            arriving: None,
            status: String::new(),
            preview: Preview::Sheet,
            size: SPECIMEN_PX,
        };
        me.measure();
        me
    }

    /// Start reading every family not read yet, three at a time, off the UI
    /// thread. `tick` collects them.
    ///
    /// Reading one means parsing it for drawing, which is most of a second
    /// for a machine's worth of fonts. Done in `rows()` -- which is where the
    /// verdicts are needed -- that was a freeze on the first `l`, with no key
    /// read until it was over.
    ///
    /// What the lanes are SET to goes first: the verdict for `nvim.editor`
    /// compares against the ui lane's font, and a lane may name a family the
    /// list below does not offer.
    fn measure(&mut self) {
        let mut names: Vec<String> = Vec::new();
        for family in self.state.iter().map(|(_, f)| f).chain(&self.families) {
            if !self.cache.contains_key(family) && !names.contains(family) {
                names.push(family.clone());
            }
        }
        if names.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let queue = Arc::new(Mutex::new(names.into_iter()));
        for _ in 0..3 {
            let (tx, queue) = (tx.clone(), Arc::clone(&queue));
            std::thread::spawn(move || loop {
                let Some(family) = queue.lock().unwrap().next() else {
                    return;
                };
                let loaded = Loaded::installed(&family);
                // The screen has gone; nobody wants the rest.
                if tx.send((family, loaded)).is_err() {
                    return;
                }
            });
        }
        self.arriving = Some(rx);
    }

    fn loaded(&self, family: &str) -> Option<&Loaded> {
        self.cache.get(family)?.as_ref()
    }

    fn ui_advance(&self) -> f32 {
        self.state
            .iter()
            .find(|(lane, _)| lane == "nvim.ui")
            .and_then(|(_, family)| self.loaded(family))
            .map_or(0.0, |l| l.metrics.advance)
    }

    /// Which rows are lane headers and which are fonts, in display order.
    fn layout(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        for lane in 0..self.state.len() {
            if lane > 0 {
                out.push(Entry::Gap);
            }
            out.push(Entry::Lane(lane));
            if self.open == Some(lane) {
                out.extend((0..self.families.len()).map(|f| Entry::Font(lane, f)));
                out.push(Entry::Browse(lane));
            }
        }
        out
    }

    /// Open `open` (or nothing) and put the cursor on `lane`'s own row.
    ///
    /// `Flow::Retarget` and not `Dirty`, because opening or closing a lane
    /// moves every row below it. `Dirty` keeps the cursor's NUMBER, which
    /// after a close was a blank spacer or some other lane -- and after
    /// opening a lower lane was that lane's browse row, where a second Enter
    /// went off to Google instead of closing it.
    fn show(&mut self, open: Option<usize>, lane: usize) -> Flow {
        self.open = open;
        self.land = self
            .layout()
            .iter()
            .position(|e| matches!(e, Entry::Lane(i) if *i == lane))
            .unwrap_or(0);
        Flow::Retarget
    }

    /// The contact sheet: every font this lane could take, each row drawn in
    /// ITS OWN face, the current one marked. Comparing by scrolling beats
    /// landing on each one in turn.
    fn sheet(&self, family: &str, cols: u16, art_rows: usize) -> Option<Image> {
        let sheet_rows = art_rows.min(self.families.len());
        if sheet_rows < 3 {
            return None;
        }
        let idx = self.families.iter().position(|f| f == family).unwrap_or(0);
        let lo = idx
            .saturating_sub(sheet_rows / 2)
            .min(self.families.len() - sheet_rows);
        // One font per terminal ROW, at the terminal's own cell size, so the
        // sheet is drawn 1:1 rather than squeezed into whatever the scaler
        // makes of it.
        let (cw, ch) = tui_core::term_info().cell;
        let row_h = ch as usize;
        let canvas_w = cols as usize * cw as usize;
        let mut c = render::Canvas::new(canvas_w, row_h * sheet_rows, theme::bg());
        let sample_x = (canvas_w as f32 * 0.42) as i32;
        let sheet_size = ch as f32 * 0.72;
        for (n, fam) in self.families[lo..lo + sheet_rows].iter().enumerate() {
            let y = n * row_h;
            if fam == family {
                c.fill(0, y, canvas_w, row_h, theme::selection());
            }
            let Some(l) = self.loaded(fam) else { continue };
            let base = (y + row_h) as i32 - (row_h as i32 / 5);
            c.text(&l.font, sheet_size, (cw as i32) * 2, base, fam, theme::fg());
            c.text(&l.font, sheet_size, sample_x, base, SAMPLE, theme::dim());
        }
        Some(Image {
            png: c.to_png()?,
            rows: sheet_rows as u16,
        })
    }
}

#[derive(Clone, Copy)]
enum Entry {
    /// A blank row between lanes. They are categories; spacing is what says so.
    Gap,
    /// A lane, by its place in `lanes::LANES`.
    Lane(usize),
    /// A family under an open lane: the lane, and the family's place in
    /// `families`.
    Font(usize, usize),
    /// The last row under an open lane. A row rather than a keybinding because
    /// the screen is supposed to explain itself — a key you have to be told
    /// about is a key nobody finds.
    Browse(usize),
}

fn verdict(lane: &str, l: &Loaded, ui_advance: f32) -> (&'static str, u8, String) {
    let m = &l.metrics;
    if !m.fixed {
        return (
            "POOR",
            210,
            "not fixed-pitch — columns will not line up".into(),
        );
    }
    match lane {
        "claude" => {
            if m.critical < fonts::CRITICAL.len() {
                (
                    "POOR",
                    210,
                    format!(
                        "missing {} — drawn on every tool call",
                        m.missing.join(", ")
                    ),
                )
            } else if m.frequent < fonts::FREQUENT.len() {
                ("OK", 221, "the per-tool-call glyphs are native".into())
            } else if m.incidental < fonts::INCIDENTAL.len() {
                (
                    "OK",
                    221,
                    "critical and frequent native; some punctuation falls back".into(),
                )
            } else {
                ("GOOD", 114, "all nine TUI glyphs native".into())
            }
        }
        "nvim.editor" => {
            let italic = l.cuts.iter().any(|c| c == "Italic");
            if !italic {
                (
                    "POOR",
                    210,
                    "no Italic cut — every comment loses its slant".into(),
                )
            } else if ui_advance > 0.0 && m.advance > ui_advance + 1e-4 {
                (
                    "POOR",
                    210,
                    format!(
                        "wider than the ui cell ({:.3}em) — glyphs collide",
                        ui_advance
                    ),
                )
            } else {
                ("GOOD", 114, "has Italic and fits the ui cell".into())
            }
        }
        _ => {
            if m.nerd {
                ("GOOD", 114, "nerd icons native, at cell width".into())
            } else {
                (
                    "OK",
                    221,
                    "no nerd glyphs — the fallback draws them oversized".into(),
                )
            }
        }
    }
}

const SAMPLE: &str = "The quick brown fox 0O1lI {}[]";

/// What the pane draws in SPECIMEN mode: the lane's actual job, in the
/// candidate face and THIS MACHINE'S colours.
///
/// A pangram tells you the letterforms and nothing about whether the font can
/// do the work. For `claude` that work is a tool-call transcript, and the two
/// glyphs it leans on — U+23FA and U+23BF — are exactly the ones most coding
/// fonts lack. Seeing them in place beats reading "critical 0/2": a fallback at
/// the wrong cell width is something you recognise instantly and would struggle
/// to predict from a number. For `shell` it is the real PS1 out of
/// ~/.bash_prompt, because a prompt you look at fifty times a day is the honest
/// test.
pub fn specimen(lane: &str) -> Vec<theme::Line> {
    let fg = theme::fg();
    let dim = theme::dim();
    let ok = theme::idx(114);
    let warm = theme::idx(221);
    let cyan = theme::idx(110);

    let line = |text: &str, c: [u8; 3]| -> theme::Line { vec![(text.to_string(), c)] };
    let blank = || -> theme::Line { vec![] };

    match lane {
        "claude" => vec![
            line("\u{2733} claude \u{00b7} ~/dotfiles", warm),
            blank(),
            line("\u{23FA} Bash(cargo build --release)", ok),
            line("  \u{23BF}  Finished `release` in 0.83s", dim),
            blank(),
            line("\u{23FA} Read(tui/font/src/main.rs)", ok),
            line("  \u{23BF}  Read 312 lines", dim),
            blank(),
            line(
                "\u{25D0} Thinking\u{2026}   \u{2610} draft  \u{2612} build  \u{2713} test",
                dim,
            ),
            blank(),
            line("> how do I try it?", fg),
        ],
        "nvim.editor" => vec![
            line("-- lanes.rs \u{00b7} 42:8", dim),
            blank(),
            line("pub fn read() -> Vec<(String, String)> {", fg),
            line(
                "    // truncate-and-write, not rename \u{2014} see the note",
                dim,
            ),
            line("    let text = fs::read_to_string(path)", fg),
            line("        .unwrap_or_default();", fg),
            line("    let name = \"0xProto Nerd Font\";", ok),
            line("    if a != b && c >= d || e <= f { .. }", fg),
            line("}", fg),
        ],
        "nvim.ui" => vec![
            line("\u{f07b} nvim  \u{f0e7} oil \u{00b7} ~/dotfiles/tui", cyan),
            blank(),
            line("  ../", dim),
            line("  \u{f115} dots/", cyan),
            line("  \u{f115} font/", cyan),
            line("  \u{f15b} Cargo.toml         412 B", fg),
            line("  \u{f15b} deps.sh            1.1 K", fg),
            blank(),
            line("  NORMAL   tui/font/src   42:8   utf-8", dim),
        ],
        // The shell lane gets the real prompt, then a session around it.
        _ => {
            let mut out = theme::prompt_lines();
            if let Some(last) = out.last_mut() {
                last.push(("eza -la --icons --git".into(), fg));
            }
            out.push(blank());
            out.push(line("  \u{f15b} deps.conf      2.1K  M", dim));
            out.push(line("  \u{f115} tui/           4.0K", cyan));
            out.push(line("  \u{f15b} Makefile       8.7K", dim));
            out.push(blank());
            out.push(line(
                "  \u{2713} font  ok   0O1lI {}[] => != === \u{f408}",
                ok,
            ));
            out
        }
    }
}

/// The specimen for `lane`, drawn in `font` to fill `art_rows` of the pane.
///
/// EXACTLY the pixels the terminal will give those cells, so the image is
/// drawn 1:1 instead of resampled. Resampled text is the blur; nothing else
/// about the rasteriser was ever the problem.
pub(crate) fn specimen_image(
    font: &fontdue::Font,
    lane: &str,
    size: f32,
    cols: u16,
    art_rows: usize,
) -> Option<Image> {
    let row_h = (size * 1.55) as usize;
    let (cw, ch) = tui_core::term_info().cell;
    let canvas_w = cols as usize * cw as usize;
    let mut c = render::Canvas::new(canvas_w, art_rows * ch as usize, theme::bg());
    for (n, runs) in specimen(lane).iter().enumerate() {
        let base = (ch as usize / 2 + n * row_h) as i32 + size as i32;
        let mut x = (cw as i32) * 2;
        for (text, color) in runs {
            x = c.text(font, size, x, base, text, *color);
        }
    }
    Some(Image {
        png: c.to_png()?,
        rows: art_rows as u16,
    })
}

/// The specimen size after `+`, `=` or `-`.
pub(crate) fn resized(size: f32, key: char) -> f32 {
    if key == '-' {
        (size - 2.0).max(10.0)
    } else {
        (size + 2.0).min(48.0)
    }
}

/// The lines both screens say about a font in the same words.
pub(crate) fn space_line(m: &fonts::Metrics) -> String {
    format!(
        "  space   {:.3}em wide · {:.3}em line · x-height {:.3}",
        m.advance, m.line, m.x_height
    )
}

/// How far apart the pairs a coding font is chosen to keep apart are drawn.
pub(crate) fn glyphs_line(font: &fontdue::Font) -> String {
    let d = |a: char, b: char| render::distinct(font, a, b, 64.0) * 100.0;
    format!(
        "  glyphs  0/O {:.0}%  1/l {:.0}%  l/I {:.0}%",
        d('0', 'O'),
        d('1', 'l'),
        d('l', 'I')
    )
}

pub(crate) fn claude_line(m: &fonts::Metrics) -> String {
    format!(
        "  claude  critical {}/2  frequent {}/2  incidental {}/5",
        m.critical, m.frequent, m.incidental
    )
}

pub(crate) fn ligatures(m: &fonts::Metrics) -> String {
    if m.ligatures.is_empty() {
        "none".into()
    } else {
        m.ligatures.join(", ")
    }
}

impl Screen for FontScreen {
    fn title(&self) -> String {
        "  fonts — the four lanes wezterm draws with".into()
    }

    fn initial_sel(&self) -> usize {
        self.land
    }

    fn tick(&mut self) -> Tick {
        let Some(rx) = &self.arriving else {
            return Tick::Idle;
        };
        let mut landed = false;
        loop {
            match rx.try_recv() {
                Ok((family, loaded)) => {
                    self.cache.insert(family, loaded);
                    landed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.arriving = None;
                    break;
                }
            }
        }
        if landed {
            Tick::Changed
        } else if self.arriving.is_some() {
            Tick::Wake(MEASURING)
        } else {
            Tick::Idle
        }
    }

    fn rows(&mut self) -> Vec<Row> {
        if std::mem::take(&mut self.reread) {
            self.families = fonts::families();
            // One that could not be read may be readable now.
            self.cache.retain(|_, loaded| loaded.is_some());
            self.measure();
        }
        let ui_adv = self.ui_advance();
        self.layout()
            .into_iter()
            .map(|e| match e {
                Entry::Gap => Row::spacer(),
                Entry::Lane(i) => {
                    // Label and value are separate columns so the label can be
                    // bold and aligned while the value stays dim behind it.
                    let mut r = Row::leaf(format!("{:<11}", LANES[i])).detail(&self.state[i].1);
                    r.group = true;
                    r.expanded = self.open == Some(i);
                    r
                }
                Entry::Browse(_) => {
                    let mut r = Row::leaf("browse Google fonts\u{2026}");
                    r.depth = 1;
                    r
                }
                Entry::Font(i, f) => {
                    let family = &self.families[f];
                    let mut r = Row::leaf(family);
                    r.depth = 1;
                    // No badge until the font has been read; it arrives.
                    if let Some(l) = self.loaded(family) {
                        let (word, colour, _) = verdict(LANES[i], l, ui_adv);
                        r = r.badged(word, colour);
                    }
                    r
                }
            })
            .collect()
    }

    fn footer(&self) -> String {
        if !self.status.is_empty() {
            return self.status.clone();
        }
        let mode = match self.preview {
            Preview::Sheet => "specimen",
            Preview::Specimen => "sheet",
        };
        format!(
            "j/k move · l open · enter set · p {mode} · +/- size {:.0} · gg/G",
            self.size
        )
    }

    fn pane(&mut self, sel: usize, cols: u16, rows: u16) -> Pane {
        let mut pane = Pane::default();
        let Some(entry) = self.layout().get(sel).copied() else {
            return pane;
        };
        let (lane, family) = match entry {
            Entry::Gap => return pane,
            Entry::Browse(i) => {
                let lane = LANES[i];
                pane.lines.push("\x1b[1mbrowse Google fonts\x1b[0m".into());
                pane.lines.push(String::new());
                pane.lines
                    .push(format!("  ~50 monospace families, judged for {lane}."));
                pane.lines.push(String::new());
                pane.lines.push(
                    "  \x1b[38;5;244mNothing is installed by looking. Each family is\x1b[0m".into(),
                );
                pane.lines.push(
                    "  \x1b[38;5;244mfetched to ~/.cache/dotfiles-fonts and previewed\x1b[0m"
                        .into(),
                );
                pane.lines.push(
                    "  \x1b[38;5;244mthere; the rows around the cursor come too, so the\x1b[0m"
                        .into(),
                );
                pane.lines
                    .push("  \x1b[38;5;244mnext one has already arrived.\x1b[0m".into());
                pane.lines.push(String::new());
                pane.lines.push(
                    "  \x1b[38;5;221mNone are Nerd-patched — icons fall back oversized.\x1b[0m"
                        .into(),
                );
                return pane;
            }
            Entry::Lane(i) => {
                let lane = LANES[i];
                pane.lines.push(format!("\x1b[1m{lane}\x1b[0m"));
                pane.lines
                    .push(format!("\x1b[38;5;244m{}\x1b[0m", lanes::covers(lane)));
                pane.lines.push(String::new());
                pane.lines.push(format!("  now: {}", self.state[i].1));
                pane.lines.push(String::new());
                pane.lines
                    .push("\x1b[38;5;244m  l or → to list the installed families\x1b[0m".into());
                return pane;
            }
            Entry::Font(i, f) => (LANES[i], self.families[f].clone()),
        };

        // One image, because the pane holds one: the lane's job in this face,
        // or every face side by side. Checked for here as well as in the
        // framework, so a terminal without inline images does not pay to
        // rasterise something it cannot show.
        let art_rows = rows.saturating_sub(16) as usize;
        if tui_core::image::supported() {
            pane.image = match self.preview {
                Preview::Specimen if art_rows >= 6 => self
                    .loaded(&family)
                    .and_then(|l| specimen_image(&l.font, lane, self.size, cols, art_rows)),
                Preview::Specimen => None,
                Preview::Sheet => self.sheet(&family, cols, art_rows),
            };
        } else {
            pane.lines.push(
                "\x1b[38;5;244m  (the contact sheet needs wezterm — metrics only here)\x1b[0m"
                    .into(),
            );
            pane.lines.push(String::new());
        }

        let Some(l) = self.loaded(&family) else {
            pane.lines.push(if self.cache.contains_key(&family) {
                format!("{family} — unreadable")
            } else {
                format!("\x1b[38;5;244m  measuring {family}…\x1b[0m")
            });
            return pane;
        };
        let m = &l.metrics;
        let (v, color, why) = verdict(lane, l, self.ui_advance());

        pane.lines.push(format!("\x1b[1m{family}\x1b[0m"));
        pane.lines.push(String::new());
        pane.lines.push(space_line(m));
        if let Some((lo, hi)) = m.variable {
            pane.lines
                .push(format!("  weight  {} · variable {lo:.0}–{hi:.0}", m.weight));
        } else {
            pane.lines.push(format!("  weight  {}", m.weight));
        }
        pane.lines.push(glyphs_line(&l.font));
        pane.lines.push(format!(
            "  extras  ligatures {} · {} glyphs",
            ligatures(m),
            m.glyphs
        ));
        pane.lines.push(format!(
            "          powerline {} · {} · box {}/128",
            m.powerline,
            if m.braille { "braille" } else { "no braille" },
            m.box_drawing
        ));
        pane.lines.push(format!("  cuts    {}", l.cuts.join(", ")));
        pane.lines.push(claude_line(m));
        pane.lines.push(String::new());
        pane.lines.push(format!(
            "  fit \x1b[38;5;{color}m{v}\x1b[0m for {lane} — {why}"
        ));
        pane
    }

    fn on_action(&mut self, action: Action, sel: usize) -> Flow {
        let Some(entry) = self.layout().get(sel).copied() else {
            return Flow::Continue;
        };
        self.status.clear();
        match (action, entry) {
            (_, Entry::Gap) => Flow::Continue,
            // `l` opens a closed lane. On an open one it has nothing left to
            // open, and answering `Continue` is what lets it move on into the
            // pane -- the same edge rule as everywhere else.
            (Action::Open, Entry::Lane(i)) if self.open != Some(i) => self.show(Some(i), i),
            (Action::Close, Entry::Lane(i) | Entry::Font(i, _) | Entry::Browse(i))
                if self.open == Some(i) =>
            {
                self.show(None, i)
            }
            (Action::Activate, Entry::Lane(i)) => {
                let open = (self.open != Some(i)).then_some(i);
                self.show(open, i)
            }
            // Enter alone. `l` only ever opens or moves right, and a second key
            // that goes off to the network is a second way to do it by accident.
            (Action::Activate, Entry::Browse(i)) => {
                self.reread = true;
                Flow::Push(Box::new(browse::BrowseScreen::new(LANES[i].to_string())))
            }
            (Action::Activate, Entry::Font(i, f)) => {
                let family = self.families[f].clone();
                let was = std::mem::replace(&mut self.state[i].1, family.clone());
                match lanes::write(&self.state) {
                    Ok(()) => self.status = format!("{} → {family}", LANES[i]),
                    Err(e) => {
                        // Not saved, so not shown as if it were.
                        self.state[i].1 = was;
                        self.status = format!("could not write: {e}");
                    }
                }
                Flow::Dirty
            }
            // Screen-local keys arrive as Action::Key rather than being added to
            // the shared table in tui_core::nav, so a key added here can never
            // change what it means in github or the dots console.
            (Action::Key('p'), _) => {
                self.preview = match self.preview {
                    Preview::Sheet => Preview::Specimen,
                    Preview::Specimen => Preview::Sheet,
                };
                Flow::Dirty
            }
            (Action::Key(key @ ('+' | '=' | '-')), _) => {
                self.size = resized(self.size, key);
                Flow::Dirty
            }
            _ => Flow::Continue,
        }
    }
}
