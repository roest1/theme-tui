//! The browse screen — try Google's monospace fonts without installing them.
//!
//! A flat list with the picker's keys and its specimen. The only new idea is
//! that a row's font may not exist yet, so the pane has a third state besides
//! "here it is" and "cannot read it": "on its way". See `fetch`.
//!
//! There is no contact sheet here. The picker's compares the families you
//! HAVE, each in its own face; these are still arriving.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use tui_core::{nav::Action, Flow, Pane, Row, Screen, Tick};

use crate::fetch::{self, Fetch, Fetcher};
use crate::fonts;
use crate::picker::{
    claude_line, glyphs_line, ligatures, resized, space_line, specimen_image, Loaded, SPECIMEN_PX,
};

/// How many rows either side to pull in. Four covers a `ctrl-d` half-page on a
/// short pane and a comfortable j-hold on a tall one; much more and a skim down
/// the list queues fifty downloads nobody asked for.
const PREFETCH_RADIUS: usize = 4;

/// How often to look for a download while one is in flight. A network round
/// trip is not a frame's work: asking every frame spun the loop at 60fps for
/// as long as curl took.
const IN_FLIGHT: Duration = Duration::from_millis(100);

/// A family that has been fetched and read: the font, and the files it came
/// from, which are what Enter installs.
struct Fetched {
    face: Loaded,
    paths: Vec<PathBuf>,
}

pub struct BrowseScreen {
    items: fetch::Catalogue,
    /// A newer catalogue on its way, when the one on disk is stale or absent.
    arriving: Option<Receiver<Option<fetch::Catalogue>>>,
    fetcher: Fetcher,
    loaded: HashMap<String, Option<Fetched>>,
    epoch_seen: u64,
    lane: String,
    sel: usize,
    size: f32,
    status: String,
}

impl BrowseScreen {
    /// Opens on whatever catalogue is on disk, at once. Fetching it here put
    /// a sixty-second curl on the UI thread: on a stalled connection the whole
    /// program stopped reading keys, ctrl-c included.
    pub fn new(lane: String) -> Self {
        let (items, fresh) = fetch::cached_catalogue();
        let arriving = (!fresh).then(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(fetch::refresh_catalogue());
            });
            rx
        });
        Self {
            items,
            arriving,
            fetcher: Fetcher::new(),
            loaded: HashMap::new(),
            epoch_seen: 0,
            lane,
            sel: 0,
            size: SPECIMEN_PX,
            status: String::new(),
        }
    }

    /// Parse on first sight, then keep it. A family's file does not change
    /// under us, and re-parsing on every frame of a specimen would undo the
    /// whole point of holding state in a process.
    fn load(&mut self, slug: &str) -> Option<&Fetched> {
        if !self.loaded.contains_key(slug) {
            let Some(Fetch::Ready(paths)) = self.fetcher.get(slug) else {
                return None;
            };
            // By file name, which is all a font fontconfig has never seen has
            // to say which cut it is. Same rule as the picker's.
            let named: Vec<(String, &PathBuf)> =
                paths.iter().map(|p| (fonts::file_name(p), p)).collect();
            let face = fonts::upright(&named).and_then(|path| {
                Loaded::read(&path.to_string_lossy(), fonts::styles_of_files(&paths))
            });
            self.loaded
                .insert(slug.to_string(), face.map(|face| Fetched { face, paths }));
        }
        self.loaded.get(slug)?.as_ref()
    }

    fn install(&mut self, slug: &str, family: &str) {
        let Some(fetched) = self.loaded.get(slug).and_then(|o| o.as_ref()) else {
            self.status = format!("{family} is not here yet");
            return;
        };
        let dest = PathBuf::from(crate::theme::home())
            .join(".local/share/fonts")
            .join(fetch::key(slug));
        if let Err(e) = std::fs::create_dir_all(&dest) {
            self.status = format!("could not create {}: {e}", dest.display());
            return;
        }
        for p in &fetched.paths {
            let Some(name) = p.file_name() else { continue };
            if let Err(e) = std::fs::copy(p, dest.join(name)) {
                self.status = format!("copy failed: {e}");
                return;
            }
        }
        let _ = std::process::Command::new("fc-cache")
            .arg("-f")
            .arg(&dest)
            .output();
        self.status = format!(
            "installed {family} → {} · not in deps.conf, so this machine only",
            dest.display()
        );
    }
}

impl Screen for BrowseScreen {
    fn title(&self) -> String {
        format!(
            "  browse — Google monospace via Bunny · judging for {}",
            self.lane
        )
    }

    fn rows(&mut self) -> Vec<Row> {
        // Offline with no cached catalogue. Handled here rather than by
        // refusing to open, so the screen can say WHY it is empty — a list
        // with no rows and no explanation is the worse answer.
        if self.items.is_empty() {
            return vec![if self.arriving.is_some() {
                Row::leaf("fetching the catalogue\u{2026}")
            } else {
                Row::leaf("catalogue unavailable").badged("offline?", 210)
            }];
        }
        let items = self.items.clone();
        items
            .iter()
            .map(|(slug, name)| {
                let row = Row::leaf(name);
                match self.fetcher.get(slug) {
                    Some(Fetch::Ready(_)) => match self.load(slug) {
                        Some(f) if f.face.metrics.critical == fonts::CRITICAL.len() => {
                            row.badged("claude ok", 114)
                        }
                        Some(_) => row.badged("ready", 244),
                        None => row.badged("unreadable", 210),
                    },
                    Some(Fetch::Pending) => row.badged("\u{2026}", 244),
                    Some(Fetch::Failed(_)) => row.badged("failed", 210),
                    None => row,
                }
            })
            .collect()
    }

    fn footer(&self) -> String {
        if !self.status.is_empty() {
            return self.status.clone();
        }
        format!(
            "j/k move · enter install · +/- size {:.0} · gg/G",
            self.size
        )
    }

    /// Prefetch is driven from here rather than from `pane`, because a row you
    /// skim past without stopping on still has to be fetched — that is what
    /// makes the next keypress instant instead of the current one slow.
    fn focus(&mut self, sel: usize) {
        self.sel = sel;
        // What the last key said belongs to the row it was pressed on.
        self.status.clear();
        // Coming BACK to a family that failed asks for it again. Only this
        // one row: see `Fetcher::retry`.
        if let Some((slug, _)) = self.items.get(sel) {
            self.fetcher.retry(slug);
        }
        for i in fetch::window(self.items.len(), sel, PREFETCH_RADIUS) {
            if let Some((slug, _)) = self.items.get(i) {
                self.fetcher.request(slug);
            }
        }
    }

    fn tick(&mut self) -> Tick {
        if let Some(rx) = &self.arriving {
            match rx.try_recv() {
                Ok(got) => {
                    self.arriving = None;
                    // A failed refresh leaves the catalogue already on show.
                    if let Some(items) = got {
                        self.items = items;
                    }
                    // Nothing was asked for while the list was empty, and no
                    // key has been pressed to ask since.
                    self.focus(self.sel);
                    return Tick::Changed;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.arriving = None,
            }
        }
        let epoch = self.fetcher.epoch();
        if epoch != self.epoch_seen {
            self.epoch_seen = epoch;
            return Tick::Changed;
        }
        if self.fetcher.pending() || self.arriving.is_some() {
            Tick::Wake(IN_FLIGHT)
        } else {
            Tick::Idle
        }
    }

    fn pane(&mut self, sel: usize, cols: u16, rows: u16) -> Pane {
        let mut pane = Pane::default();
        let Some((slug, family)) = self.items.get(sel).cloned() else {
            if self.arriving.is_some() {
                pane.lines.push(
                    "  \x1b[38;5;244mfetching the catalogue from fonts.bunny.net…\x1b[0m".into(),
                );
                return pane;
            }
            pane.lines
                .push("\x1b[1mcatalogue unavailable\x1b[0m".into());
            pane.lines.push(String::new());
            pane.lines
                .push("  Could not reach fonts.bunny.net, and nothing is cached.".into());
            pane.lines.push(String::new());
            pane.lines.push(
                "  \x1b[38;5;244mThe catalogue is one JSON, cached for a week under\x1b[0m".into(),
            );
            pane.lines
                .push("  \x1b[38;5;244m~/.cache/dotfiles-fonts. q goes back.\x1b[0m".into());
            return pane;
        };

        let fetch = self.fetcher.get(&slug);
        if !matches!(fetch, Some(Fetch::Ready(_))) {
            pane.lines.push(format!("\x1b[1m{family}\x1b[0m"));
            pane.lines.push(String::new());
            if let Some(Fetch::Failed(e)) = fetch {
                pane.lines.push(format!("  could not fetch: {e}"));
                return pane;
            }
            pane.lines
                .push("  \x1b[38;5;244mfetching from google/fonts…\x1b[0m".into());
            pane.lines.push(String::new());
            pane.lines.push(
                "  \x1b[38;5;244mNothing is installed by this — the file lands in\x1b[0m".into(),
            );
            pane.lines
                .push("  \x1b[38;5;244m~/.cache/dotfiles-fonts and is judged there.\x1b[0m".into());
            return pane;
        }

        let (lane, size) = (self.lane.clone(), self.size);
        let Some(fetched) = self.load(&slug) else {
            pane.lines.push(format!("{family} — unreadable"));
            return pane;
        };
        let (m, font) = (&fetched.face.metrics, &fetched.face.font);
        let art_rows = rows.saturating_sub(14) as usize;
        if art_rows >= 6 && tui_core::image::supported() {
            pane.image = specimen_image(font, &lane, size, cols, art_rows);
        }
        pane.lines.push(format!("\x1b[1m{family}\x1b[0m"));
        pane.lines.push(String::new());
        pane.lines.push(space_line(m));
        pane.lines.push(glyphs_line(font));
        pane.lines
            .push(format!("  cuts    {}", fetched.face.cuts.join(", ")));
        pane.lines.push(claude_line(m));
        pane.lines.push(format!(
            "  extras  ligatures {} · {} glyphs · box {}/128",
            ligatures(m),
            m.glyphs,
            m.box_drawing
        ));
        pane.lines.push(String::new());
        // Not Nerd-patched, none of them, and it is the one thing the numbers
        // above do not shout: nerd icons will fall back oversized.
        pane.lines
            .push("  \x1b[38;5;221mno nerd glyphs — icons fall back oversized\x1b[0m".into());
        pane.lines
            .push("  \x1b[38;5;244menter installs to ~/.local/share/fonts\x1b[0m".into());
        pane
    }

    fn on_action(&mut self, action: Action, sel: usize) -> Flow {
        let Some((slug, family)) = self.items.get(sel).cloned() else {
            return Flow::Continue;
        };
        match action {
            Action::Activate => {
                self.install(&slug, &family);
                Flow::Dirty
            }
            Action::Key(key @ ('+' | '=' | '-')) => {
                self.size = resized(self.size, key);
                Flow::Dirty
            }
            _ => Flow::Continue,
        }
    }
}
