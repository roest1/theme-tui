# CLAUDE.md — theme-tui

`theme`: pick the font and the colors for each of wezterm's lanes, from one place
(roest1/dotfiles#112). Icons are the part still to come. Built on
[roest1/tui-core](https://github.com/roest1/tui-core) and pinned in
[roest1/dotfiles](https://github.com/roest1/dotfiles) as a submodule at `tui/theme`, where
`dots` pushes `theme::screen()` onto its own stack, so this crate's lib is a contract, not
just a binary. Split out of dotfiles on 2026-11-23 with its history. The entries below moved
from dotfiles' `tui/CLAUDE.md` as they were; trim them here.

The command and the crate were `font` until 2026-11-24, when it had been setting colors for
a day. Three things kept the old word because they are still about fonts: `FontScreen`, the
`fonts` module, and the download cache at `~/.cache/dotfiles-fonts`. `src/theme.rs` is older
than the rename and is not the crate's root: it is the terminal's own colors and the prompt.

## Layout

- `src/` — three screens, and a LIB plus a thin bin. `lib` (the module index,
  the crate-root re-exports, and `screen()`: the two tabs), `picker` (the font
  lanes), `colors` (the color lanes), `conf` (the machine-local `theme.conf`,
  the one file every pick goes in), `lanes` (the lanes and their defaults),
  `schemes` (what there is to pick a color from: wezterm's theme files and
  nvim's colorschemes, `schemes.lua` being what nvim is asked with), `fonts`
  (discovery + measurement), `render` (rasterising, the contact sheet,
  glyph-distinctness), `fetch` (the prefetching cache behind the browse
  screen), `browse` (the Google-fonts screen), `show` (`theme show` — the lane
  report and the carrier), `main` (argument handling and nothing else).

## Things not to undo

- **Prefetch is driven by `Screen::focus`, not by `pane`.** A row you skim past
  without stopping on still has to be fetched — that is precisely what makes the
  NEXT keypress instant rather than the current one slow. `pane` only ever sees
  where you stopped.

- **`Fetcher::request` checks the disk synchronously.** Leaving that to the
  worker meant a family cached from a previous session still went Pending →
  queue → thread → Ready, so moving across fonts you had already fetched
  flashed "fetching…" every time. A `read_dir` costs microseconds.

- **You cannot rasterise a font you do not have.** "Preview without
  downloading" is not a thing; preview without INSTALLING is, and that is what
  the cache is for. Nothing reaches `~/.local/share/fonts` without Enter.

- **There is ONE implementation of "measure a font", `fonts::measure`.** The
  Python previewer that used to carry a second one was deleted because the two
  had already drifted on which file counted as Regular. (This bullet used to
  name a `font inspect` subcommand and a `fontbrowse` bash caller; neither
  exists — the command surface is the three below, and browsing is a row inside
  the picker. The principle outlived both spellings.)

- **`theme show`'s carrier is the only part of this crate that does not
  rasterise.** Everything else here draws type by decoding the file with fontdue
  and shipping a PNG, which is why the picker looked perfect on a machine where
  the lanes were dead: a rasterised specimen is a picture of what this process
  thinks the font is, not of what wezterm did with it. The carrier prints real
  SGR 6 and SGR 5 text and lets the terminal answer. Do not "unify" it with the
  rendering path — the whole value is that it goes through the code being tested.

  It replaced a version floor (`wezterm/MIN_VERSION`) that inferred the same
  answer from a build stamp. That could not fail safe: every build at or above
  the floor passed untested. Don't bring it back — `status_fonts` in
  `lib/status.sh` asks the provable half, and this asks your eyes.

  Each sample resets with `\x1b[22;25m` immediately after itself, so a
  truncated line cannot leave the terminal in SGR 6 and silently reskin
  everything printed after it.

- **`theme show` exits non-zero ONLY for a lane naming a family this machine does
  not have**, because that is the only thing here a program can judge.
  `status_fonts` counts that as drift and counts nothing else, so `make status`
  stays green over ssh, in a pipe and on a non-wezterm terminal — all places the
  carrier is unreadable and the machine is fine.

- **The picker reads fonts off the UI thread.** `FontScreen::measure` starts three
  workers and `tick` collects what they send. Reading a family means parsing it for
  drawing, about three quarters of a second for a machine's worth; done in `rows()`
  it froze the first `l`. A family not in `cache` yet is still on its way and the
  pane says "measuring"; `Some(None)` is one that could not be read.

- **Opening or closing a lane answers `Flow::Retarget`.** Every row below it moves,
  and `Flow::Dirty` keeps the cursor's number: after a close that was a blank row,
  and after opening a lower lane it was that lane's browse row.

- **The upright cut is chosen by `fonts::upright`, for both screens.** The picker
  asks it with fontconfig's style names and the browse screen with file names.
  They had one rule each and had drifted, which is the failure the bullet above
  about `fonts::measure` records.

- **A download lands in `<slug>.part` and is renamed into place.** `cached_paths`
  takes any directory holding a .ttf as a whole family, so an interrupted
  download written straight into it was a complete family for ever after.

- **The catalogue is cached only if it parses, and never fetched on the UI
  thread.** A captive portal answers with HTML and a 200; caching that was a week
  of "catalogue unavailable". Fetching it in `BrowseScreen::new` was a
  sixty-second curl with no key read.

- **`theme show` asks `fonts::has`, not the picker's list.** The list leaves out
  names such as "JetBrainsMono NF" that wezterm resolves, so asking it reported a
  healthy machine as drifted.

- **One file, `~/.config/dotfiles/theme.conf`, keyed by lane and then by what
  is set.** `shell.font`, `shell.colors`, `nvim.editor.colors`. The fonts had a
  file of their own (`~/.config/wezterm/fonts.conf`, bare lane names) and nvim's
  colors had this one to themselves (`nvim.ui = scheme`); `nvim.ui` meant a font
  in one and a colorscheme in the other, which is why every key now says its
  attribute. `conf::merged` folds both old spellings in, and the first write
  puts everything in the new one and removes `fonts.conf`: left behind, wezterm
  reads it underneath and `theme reset` would fall back to it, not to the
  defaults.

- **A write is read, change, write: `conf::update`.** Two screens write the
  file and each holds only its own picks, so neither may write "the file as I
  know it". It is also what keeps a key this program does not know: a reader can
  gain a key before the picker learns to set it, and a pick must not delete it.
  `a_key_nobody_here_knows_is_written_back` holds that.

- **`conf::parse` takes exactly the lines the Lua readers take.** Their pattern
  is in its doc comment. A line this accepted and they skipped would be a pick
  that is written, shown as set, and never applied.

- **Nothing here knows what a theme looks like.** The shell lane's palettes are
  the files in `~/.config/wezterm/themes`, which is what wezterm.lua reads the
  picked one out of. nvim's colorschemes are asked of nvim: `schemes.lua` runs
  in a headless nvim with the user's config and prints each scheme's highlight
  colors. A list in this crate would be a second copy of both. Same principle as
  `fonts::measure`, and as the colors absorbed from the terminal below.

  The script loads a scheme through the config's own loader when it publishes
  one (dotfiles' `external.theme.load`), so the preview goes through the code
  that will draw the lane. That is how rose-pine's background shows up in it at
  all: its option is set by that loader, not by `:colorscheme`.

  It goes in through an environment variable and `loadstring`, so there is no
  temp file and nothing to quote. nvim gets `-i NONE -n`: it is there to answer
  a question, not to write shada. It is asked on a thread; a tenth of a second
  is still a frozen first frame.

- **The colors screen reads the file the way the readers do.** `colors::now` is
  the same rules as wezterm.lua and nvim's `theme.lua`: only `false` is not
  transparent, an opacity outside 0.3 to 1 is opaque, an editor key left out
  follows the ui lane. The screen says what will happen, not what was typed.

- **A theme wezterm would refuse is not offered.** `Palette::parse` is whole or
  nothing, and `palettes()` skips a name that is not letters, digits, `_` and
  `-`, both as wezterm.lua does. Otherwise the list holds picks that do nothing.

- **One Dark is set opaque, and that is a taste kept here.** dotfiles tried it
  transparent and rejected it; the readers do not enforce that, on purpose.
  `lanes::opaque_only` is the whole of it: picking it writes
  `transparent = false`, and its background row will not switch back.

- **A preview line is cut to the pane by `paint`, to the cell.** Left to the
  pane's own truncation a colored block loses its right edge a line at a time.

- **Fonts and colors are tabs (`screen()`), not one tree.** They are siblings:
  the same lanes asked a different question. `dots` pushes the same host, so a
  third thing to set is a tab added in `lib.rs` and nowhere else.

- **A screen's type is public; everything else can stay crate-private.**
  `lib.rs` re-exports `FontScreen`, `Preview` and `specimen` at the crate ROOT
  rather than making callers reach into `picker::`, because that is where they
  already lived when this was one file. Keeping the paths identical is what
  made the lib/bin split a move rather than a rewrite.

  Making `new()` public turned on `clippy::new_without_default`, which only
  fires for public items — so the split surfaced two real omissions rather than
  inventing them. They got `Default` impls, not `#[allow]`: a type whose `new()`
  takes no arguments and cannot fail *is* `Default`, and saying so is the fix.

## The command surface is three commands

`theme`, `theme show`, `theme reset`, and that did not grow when colors arrived:
they are a tab. Everything else lives inside the TUI, which
states its own keys in the footer — including browsing, which is the last row
under an opened lane rather than a subcommand or a keybinding. A key you have
to be told about is a key nobody finds, and a subcommand is worse: it needs a
man page to discover something that could have been a row.

`app` keeps a screen STACK for this. `Flow::Push` opens one on top, `q` pops
below the root and quits at it, and each frame carries its own cursor, scroll
offset and reticle — so coming back lands on the row you left. That is the same
property github gets from running one fzf inside another, and it is the reason
github can move here without losing it.

## Theme, absorbed rather than parsed

Colours come from the terminal, not from `wezterm/wezterm.lua`: `OSC 11`, `OSC
10` and `OSC 4` return what it actually resolved, so a theme change is picked
up with nothing here aware that a theme exists. Parsing the Lua would have been
a second implementation of somebody else's format, wrong the first time that
file is rearranged.

What was absorbed is the terminal as it was when `theme` started. A palette
picked on the colors tab repaints the terminal at once, and the font specimens
keep the colors they were drawn with until the next start.

Indices 16-255 are **computed** locally (`theme::xterm256`) rather than
required from the terminal — the 6x6x6 cube and the grey ramp are fixed by
xterm. That is what keeps the query an improvement instead of a dependency:
falling back to the foreground colour made the prompt specimen monochrome on a
quiet terminal, and only a test asserting the runs *differed* caught it.

The shell specimen renders **this machine's PS1**, as bash expands it: a
non-interactive bash sources `~/.bash_local` (where `PS1_COLOR_*` overrides
live) and then `~/.bash_prompt`, fills the runtime-only slots by hand, and
prints `${PS1@P}`. It does not parse the `PS1='…'` line any more, because since
roest1/PS1#2 that line names colour variables, not colours. The unit test runs
the same expansion over roest1/PS1's `ps1.bash` (`PS1_BASH`, which CI sets to a fresh
clone, else the dotfiles submodule two levels up), so a PS1 change that breaks the
specimen fails `cargo test` in CI.
