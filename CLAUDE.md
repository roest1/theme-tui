# CLAUDE.md — theme-tui

`font` today: pick the font for each of wezterm's lanes. It becomes the `theme` TUI
(roest1/dotfiles#112): colors, icons and fonts per lane, from one place. Built on
[roest1/tui-core](https://github.com/roest1/tui-core) and pinned in
[roest1/dotfiles](https://github.com/roest1/dotfiles) as a submodule at `tui/font`, where
`dots` pushes `FontScreen` onto its own stack, so this crate's lib is a contract, not just a
binary. Split out of dotfiles on 2026-11-23 with its history. The entries below moved from
dotfiles' `tui/CLAUDE.md` as they were; trim them here.

## Layout

- `src/` — two screens, and a LIB plus a thin bin. `lib` (the module index
  and the crate-root re-exports), `picker` (the lane tree — the root screen),
  `fonts` (discovery + measurement), `render` (rasterising, the contact sheet,
  glyph-distinctness), `lanes` (the machine-local `fonts.conf`), `fetch` (the
  prefetching cache behind the browse screen), `browse` (the Google-fonts
  screen), `show` (`font show` — the lane report and the carrier), `main`
  (argument handling and nothing else).

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

- **`font show`'s carrier is the only part of this crate that does not
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

- **`font show` exits non-zero ONLY for a lane naming a family this machine does
  not have**, because that is the only thing here a program can judge.
  `status_fonts` counts that as drift and counts nothing else, so `make status`
  stays green over ssh, in a pipe and on a non-wezterm terminal — all places the
  carrier is unreadable and the machine is fine.

- **A screen's type is public; everything else can stay crate-private.**
  `lib.rs` re-exports `FontScreen`, `Preview` and `specimen` at the crate ROOT
  rather than making callers reach into `picker::`, because that is where they
  already lived when this was one file and `browse` addresses them as
  `crate::specimen` / `crate::Preview`. Keeping the paths identical is what
  made the lib/bin split a move rather than a rewrite.

  Making `new()` public turned on `clippy::new_without_default`, which only
  fires for public items — so the split surfaced two real omissions rather than
  inventing them. They got `Default` impls, not `#[allow]`: a type whose `new()`
  takes no arguments and cannot fail *is* `Default`, and saying so is the fix.

## The command surface is three commands

`font`, `font show`, `font reset`. Everything else lives inside the TUI, which
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
