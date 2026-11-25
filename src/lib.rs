//! `theme` — pick the font and the colors for each of wezterm's lanes.
//!
//! A LIBRARY as well as a binary, and the library half is the point. `tui-core`
//! takes a `Box<dyn Screen>` from anywhere, so a screen is reusable across
//! binaries: `theme` runs the picker as its root, and the `dots` console pushes
//! this same `FontScreen` as a row in its tree. One implementation, two
//! doors — the alternative was a second font screen inside the console, which
//! is how the two would drift.
//!
//! This does NOT make the binaries modes of one program. `theme` still starts in
//! ~1ms and links only what a picker needs; the console is a separate binary
//! that happens to depend on this one. See CLAUDE.md.

pub mod browse;
pub mod colors;
pub mod conf;
pub mod fetch;
pub mod fonts;
pub mod lanes;
pub mod render;
pub mod schemes;
pub mod show;
pub mod theme;

mod picker;

// Re-exported at the crate ROOT because that is where they already lived when
// this was one file. Keeping the paths identical is what made the split a
// move rather than a rewrite.
pub use colors::ColorScreen;
pub use picker::{specimen, FontScreen, Preview};

/// Everything a lane can be given, as tabs: its font, and its colors.
///
/// Tabs and not one tree, because the two are siblings: the same lanes, asked
/// a different question. This is what `theme` runs and what `dots` pushes, so
/// a third thing to set is a tab added here and nowhere else.
pub fn screen() -> tui_core::Host {
    tui_core::Host::new(
        None,
        vec![
            (
                "\u{1f524} Fonts".into(),
                Box::new(FontScreen::new()) as Box<dyn tui_core::Screen>,
            ),
            ("\u{1f3a8} Colors".into(), Box::new(ColorScreen::new())),
        ],
    )
}
