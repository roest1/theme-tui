# theme-tui

`font`: pick the font for each of WezTerm's lanes (the shell, nvim's chrome, the file you're editing, Claude Code panes), with a live rasterized preview of each. It's growing into a `theme` TUI that sets colors, icons and fonts per lane from one place ([roest1/dotfiles#112](https://github.com/roest1/dotfiles/issues/112)).

- `font` opens the picker. `font show` prints each lane's font, and `font reset` goes back to the defaults.
- Picks are machine-local: they go in `~/.config/wezterm/fonts.conf`, which the [roest1/dotfiles](https://github.com/roest1/dotfiles) WezTerm config reads. Nothing is installed into the system font path.
- The last row under a lane browses Google Fonts. A family is fetched to `~/.cache/dotfiles-fonts` and previewed before you choose it.

Built on [roest1/tui-core](https://github.com/roest1/tui-core). dotfiles pins it as a submodule at `tui/font` and installs it with `make install tui`. Its `dots` console opens the same picker as a row, so this crate is a library as well as a binary.

## Install

Needs a Rust toolchain. The lanes only mean something in WezTerm running the dotfiles config.

```bash
cargo install --locked --git https://github.com/roest1/theme-tui
```

## Develop

The prompt-preview test renders the real prompt from [roest1/PS1](https://github.com/roest1/PS1), so point it at a checkout:

```bash
git clone --depth 1 https://github.com/roest1/PS1.git ~/github/PS1
PS1_BASH=~/github/PS1/ps1.bash cargo test
```

Inside dotfiles it finds the `ps1` submodule on its own. To work on tui-core at the same time:

```bash
cargo build --config "patch.'https://github.com/roest1/tui-core'.tui-core.path='$HOME/github/tui-core'"
```

That rewrites `Cargo.lock` to the unpinned path, so run `git checkout Cargo.lock` before committing. CI's `--locked` build fails on a lock left that way.

## License

MIT. See [LICENSE](LICENSE).
