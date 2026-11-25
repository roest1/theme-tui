//! Fetching fonts you have not installed, ahead of when you look at them.
//!
//! ─── Why there is a cache at all ─────────────────────────────────────────
//!
//! You cannot rasterise a font you do not have. There is no metadata that
//! substitutes for outlines, so "preview without downloading" is not a thing
//! that exists — but "preview without INSTALLING" is, and it is what you
//! actually want. Candidates land in ~/.cache and are judged from there;
//! nothing touches ~/.local/share/fonts until you say so.
//!
//! ─── Why it prefetches neighbours ────────────────────────────────────────
//!
//! Fetching only what the cursor is on makes every j a stall. Fetching the
//! rows AROUND it means the ones you are about to reach are already there:
//! by the time you have read one specimen, its neighbours have arrived. The
//! order is nearest-first (i, i+1, i-1, i+2, …) because the next keystroke is
//! far more likely to be one row away than five.
//!
//! ─── Why curl and not an HTTP crate ──────────────────────────────────────
//!
//! curl is already a dependency of this repo and of the machine. Pulling in
//! reqwest would add a TLS stack and most of a runtime to a binary whose whole
//! appeal is that it starts in a millisecond.
//!
//! ─── Two sources, and the split is load-bearing ──────────────────────────
//!
//! Bunny (fonts.bunny.net) is the CATALOGUE: one JSON, no API key, with the
//! category and weights and styles to filter on. google/fonts on GitHub is the
//! FONT, because Bunny only ships woff2 subset for the web — its Fira Code
//! "latin" file has 226 glyphs and NO BOX DRAWING, against 1551 with box
//! drawing from google/fonts. Measured, not assumed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Debug)]
pub enum Fetch {
    Pending,
    Ready(Vec<PathBuf>),
    Failed(String),
}

/// What each family came to, and how many answers have landed.
struct Shared {
    state: Mutex<HashMap<String, Fetch>>,
    /// Bumped whenever an answer lands, so the UI can tell "something landed"
    /// from "still waiting" without diffing the whole map.
    epoch: AtomicU64,
}

impl Shared {
    /// Record an answer AND say that one arrived.
    ///
    /// The only way an answer gets in, so the two cannot come apart. They had:
    /// a family found already on disk was recorded without the count moving,
    /// and with a warm cache no row in the list ever got its badge.
    fn settle(&self, slug: &str, fetch: Fetch) {
        self.state.lock().unwrap().insert(slug.to_string(), fetch);
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }
}

pub struct Fetcher {
    tx: Sender<String>,
    shared: Arc<Shared>,
}

pub fn cache_dir() -> PathBuf {
    crate::fonts::xdg("XDG_CACHE_HOME", ".cache").join("dotfiles-fonts")
}

/// Where one family's files are kept until, and unless, they are installed.
fn family_dir(slug: &str) -> PathBuf {
    cache_dir().join("browse").join(slug)
}

/// Bunny's slug as google/fonts spells the directory: without its hyphens
/// (fira-code -> firacode). Checked against the repo's full tree: 50 of the 51
/// monospace families resolve, and the one that does not is material-symbols,
/// an icon set rather than a text font.
pub(crate) fn key(slug: &str) -> String {
    slug.replace('-', "")
}

/// GET `url`. On failure, the status it was refused with -- or `None` when no
/// answer came at all.
///
/// The status is asked for as DATA (`-w`), not read out of curl's complaint:
/// a sentence on stderr is a thing that gets reworded.
fn get(url: &str, seconds: &str, headers: &[&str]) -> Result<Vec<u8>, Option<u16>> {
    let mut curl = Command::new("curl");
    curl.args(["-sSL", "--max-time", seconds, "-w", "\n%{http_code}"]);
    for header in headers {
        curl.args(["-H", header]);
    }
    let out = curl.arg(url).output().map_err(|_| None)?;
    let (body, code) = out
        .stdout
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|at| out.stdout.split_at(at))
        .ok_or(None)?;
    let code: u16 = String::from_utf8_lossy(code).trim().parse().unwrap_or(0);
    if out.status.success() && (200..300).contains(&code) {
        Ok(body.to_vec())
    } else {
        // curl prints 000 for a request that never got an answer.
        Err((code != 0).then_some(code))
    }
}

/// A family's .ttf files in google/fonts.
///
/// A family that is simply not there is a 404 in all three licence
/// directories. Anything ELSE that went wrong is said, because "no .ttf for
/// fira-code" is a lie when the truth is that GitHub stopped answering: the
/// contents API allows an anonymous address sixty requests an hour, and one
/// skim down the list spends them.
fn google_ttf_urls(slug: &str) -> Result<Vec<String>, String> {
    let mut trouble = None;
    for licence in ["ofl", "apache", "ufl"] {
        let api = format!(
            "https://api.github.com/repos/google/fonts/contents/{licence}/{}",
            key(slug)
        );
        let headers = [
            "Accept: application/vnd.github+json",
            "User-Agent: dotfiles-font-tui",
        ];
        let body = match get(&api, "30", &headers) {
            Ok(body) => body,
            Err(status) => {
                trouble = trouble.or(refusal(status));
                continue;
            }
        };
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) else {
            continue;
        };
        let Some(items) = json.as_array() else {
            continue;
        };
        let urls: Vec<String> = items
            .iter()
            .filter(|i| {
                i.get("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n.ends_with(".ttf"))
            })
            .filter_map(|i| i.get("download_url")?.as_str().map(str::to_string))
            .collect();
        if !urls.is_empty() {
            return Ok(urls);
        }
    }
    Err(trouble.unwrap_or_else(|| format!("no .ttf for {slug}")))
}

/// What a failed request means, unless it only means "not in this directory".
fn refusal(status: Option<u16>) -> Option<String> {
    match status {
        Some(404) => None,
        Some(403 | 429) => {
            Some("github's hourly limit for anonymous requests is used up — try again later".into())
        }
        Some(code) => Some(format!("github answered {code}")),
        None => Some("could not reach github".into()),
    }
}

/// What is already on disk for this family, if anything.
///
/// Checked SYNCHRONOUSLY by `request`, not just by the worker. Leaving it to
/// the worker meant a family cached from a previous session still went through
/// Pending -> queue -> thread -> Ready, so moving the cursor across fonts you
/// had already fetched still flashed "fetching…". A read_dir costs microseconds
/// and removes the flash entirely.
fn cached_paths(slug: &str) -> Option<Vec<PathBuf>> {
    let found: Vec<PathBuf> = std::fs::read_dir(family_dir(slug))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ttf"))
        .collect();
    (!found.is_empty()).then_some(found)
}

fn fetch_one(slug: &str) -> Result<Vec<PathBuf>, String> {
    if let Some(found) = cached_paths(slug) {
        return Ok(found);
    }

    let urls = google_ttf_urls(slug)?;
    // Into a directory of its own, moved into place only once EVERY file has
    // landed. `cached_paths` takes any directory holding a .ttf as a whole
    // family, so writing straight into it meant a download interrupted after
    // one cut of four was a complete family for ever after -- measured from
    // whichever cut that was, and installed as that one file.
    let dir = family_dir(slug);
    let part = dir.with_extension("part");
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| e.to_string())?;
    for url in urls {
        let name = url.rsplit('/').next().unwrap_or("font.ttf");
        let bytes = get(&url, "60", &[]).map_err(|_| format!("download failed: {name}"))?;
        std::fs::write(part.join(name), bytes).map_err(|e| e.to_string())?;
    }
    // Another `theme` may have finished the same family first, in which case
    // the rename fails and what it left is the answer.
    if std::fs::rename(&part, &dir).is_err() {
        let _ = std::fs::remove_dir_all(&part);
    }
    cached_paths(slug).ok_or_else(|| format!("could not keep {slug} in the cache"))
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher {
    /// Three workers: enough that a stalled request does not block the queue,
    /// few enough to stay polite to two public APIs.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<String>();
        let shared = Arc::new(Shared {
            state: Mutex::new(HashMap::new()),
            epoch: AtomicU64::new(0),
        });
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..3 {
            let rx = Arc::clone(&rx);
            let shared = Arc::clone(&shared);
            thread::spawn(move || loop {
                let slug = {
                    let guard = rx.lock().unwrap();
                    match guard.recv() {
                        Ok(s) => s,
                        Err(_) => return,
                    }
                };
                let result = match fetch_one(&slug) {
                    Ok(paths) => Fetch::Ready(paths),
                    Err(e) => Fetch::Failed(e),
                };
                shared.settle(&slug, result);
            });
        }
        Self { tx, shared }
    }

    /// How many answers have landed. A screen that sees it move re-reads.
    pub fn epoch(&self) -> u64 {
        self.shared.epoch.load(Ordering::Relaxed)
    }

    pub fn get(&self, slug: &str) -> Option<Fetch> {
        self.shared.state.lock().unwrap().get(slug).cloned()
    }

    /// Ask for a family, if this is the first time we have been asked.
    /// Idempotent, so a caller can request the same window on every cursor
    /// move without re-downloading anything.
    pub fn request(&self, slug: &str) {
        if self.get(slug).is_some() {
            return;
        }
        // Already downloaded in an earlier session: resolve it here rather than
        // queueing, so a warm cache never shows a spinner it does not need.
        match cached_paths(slug) {
            Some(found) => self.shared.settle(slug, Fetch::Ready(found)),
            None => self.enqueue(slug),
        }
    }

    /// Ask again for a family that failed.
    ///
    /// `request` leaves a failure alone, deliberately: it is called for every
    /// row near the cursor on every move, and re-asking each time would spend
    /// the very limit that caused most failures. This is for the ONE row the
    /// cursor has come back to.
    pub fn retry(&self, slug: &str) {
        if matches!(self.get(slug), Some(Fetch::Failed(_))) {
            self.enqueue(slug);
        }
    }

    fn enqueue(&self, slug: &str) {
        let mut state = self.shared.state.lock().unwrap();
        state.insert(slug.to_string(), Fetch::Pending);
        drop(state);
        let _ = self.tx.send(slug.to_string());
    }

    pub fn pending(&self) -> bool {
        self.shared
            .state
            .lock()
            .unwrap()
            .values()
            .any(|f| matches!(f, Fetch::Pending))
    }
}

/// (slug, family name) for each family on offer.
pub type Catalogue = Vec<(String, String)>;

fn catalogue_path() -> PathBuf {
    cache_dir().join("bunny-list.json")
}

/// The catalogue as it is on disk, whatever its age, and whether it is still
/// FRESH -- under a week old, so there is nothing to go and fetch.
///
/// Never touches the network, which is what lets the browse screen open at
/// once. Offline with a stale catalogue beats offline with nothing, so a stale
/// one is returned too; `refresh_catalogue` is what replaces it.
pub fn cached_catalogue() -> (Catalogue, bool) {
    let path = catalogue_path();
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < 7 * 24 * 3600);
    let items = std::fs::read(&path)
        .ok()
        .and_then(|body| parse_catalogue(&body));
    // A file that does not parse is not a catalogue, however new it is.
    let fresh = fresh && items.is_some();
    (items.unwrap_or_default(), fresh)
}

/// Fetch the catalogue and keep it. BLOCKS on the network for as long as curl
/// allows, so it is called from a worker and never from a screen.
///
/// Kept only if it PARSES. A captive portal answers any URL with a page of
/// HTML and a 200, and caching that first meant a week of "catalogue
/// unavailable" on a network that was fine.
pub fn refresh_catalogue() -> Option<Catalogue> {
    let body = get("https://fonts.bunny.net/list", "60", &[]).ok()?;
    let items = parse_catalogue(&body)?;
    let path = catalogue_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // tmp-then-rename here, unlike theme.conf (see `conf::update`): nothing watches this
    // file, so there is no inode to preserve, and a half-written catalogue
    // replacing a good one would be the worse failure.
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &body).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    Some(items)
}

/// (slug, familyName) for every monospace family, minus material-symbols —
/// categorised monospace, but an icon set, and the one slug that does not
/// resolve in google/fonts. `None` for anything that is not the catalogue.
fn parse_catalogue(body: &[u8]) -> Option<Catalogue> {
    let json = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    let map = json.as_object()?;
    let mut out: Catalogue = map
        .iter()
        .filter(|(slug, v)| {
            slug.as_str() != "material-symbols"
                && v.get("category").and_then(|c| c.as_str()) == Some("monospace")
        })
        .map(|(slug, v)| {
            let name = v
                .get("familyName")
                .and_then(|n| n.as_str())
                .unwrap_or(slug)
                .to_string();
            (slug.clone(), name)
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Some(out)
}

/// Nearest-first, so the row you are about to reach is fetched before the row
/// five away. `radius` is how far either side to reach.
pub fn window(len: usize, sel: usize, radius: usize) -> Vec<usize> {
    let mut out = vec![sel.min(len.saturating_sub(1))];
    for d in 1..=radius {
        if sel + d < len {
            out.push(sel + d);
        }
        if let Some(i) = sel.checked_sub(d) {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{parse_catalogue, refusal, window};

    /// The body a captive portal sends is a 200 full of HTML. It must not be
    /// taken for a catalogue -- that is what used to get cached for a week.
    #[test]
    fn only_a_catalogue_parses_as_one() {
        assert_eq!(parse_catalogue(b"<html>sign in to the wifi</html>"), None);
        assert_eq!(parse_catalogue(b"[1, 2]"), None);
        let body = br#"{
            "fira-code": {"category": "monospace", "familyName": "Fira Code"},
            "inter": {"category": "sans-serif", "familyName": "Inter"},
            "material-symbols": {"category": "monospace", "familyName": "Material Symbols"}
        }"#;
        assert_eq!(
            parse_catalogue(body),
            Some(vec![("fira-code".to_string(), "Fira Code".to_string())])
        );
    }

    /// "Not in this licence directory" is the ordinary answer and says
    /// nothing. Everything else is why the family could not be fetched, and a
    /// spent rate limit must not be reported as a font that does not exist.
    #[test]
    fn a_refusal_is_told_from_an_absence() {
        assert_eq!(refusal(Some(404)), None);
        assert!(refusal(Some(403)).unwrap().contains("hourly limit"));
        assert!(refusal(Some(429)).unwrap().contains("hourly limit"));
        assert_eq!(refusal(Some(500)).unwrap(), "github answered 500");
        assert_eq!(refusal(None).unwrap(), "could not reach github");
    }

    #[test]
    fn prefetch_order_is_nearest_first() {
        assert_eq!(window(20, 10, 3), vec![10, 11, 9, 12, 8, 13, 7]);
    }

    /// The ends are where an off-by-one shows up as a panic rather than a
    /// missing preview.
    #[test]
    fn edges_do_not_run_off_the_list() {
        assert_eq!(window(5, 0, 3), vec![0, 1, 2, 3]);
        assert_eq!(window(5, 4, 3), vec![4, 3, 2, 1]);
        assert_eq!(window(1, 0, 3), vec![0]);
        assert_eq!(window(0, 0, 2), vec![0]);
    }
}
