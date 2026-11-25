# theme-tui

`theme`: pick the colors and the font for each of WezTerm's lanes (the shell, nvim's chrome, the file you're editing, Claude Code panes), and the icon set files are drawn with, with a preview of each.

- `theme` opens the picker. It has three tabs: Themes (the colors), Fonts and Icons. `theme show` prints what each lane is set to, and `theme reset` goes back to the defaults.
- Picks are machine-local: they go in `~/.config/dotfiles/theme.conf`, which the [roest1/dotfiles](https://github.com/roest1/dotfiles) WezTerm and nvim configs read. A pick applies at once, in the terminal and in every running nvim. Nothing is installed into the system font path.
- The last row under a font lane browses Google Fonts. A family is fetched to `~/.cache/dotfiles-fonts` and previewed before you choose it.
- Icons are Nerd Font glyphs by default, or Material Icon Theme in color where the terminal has that font ([roest1/fonts](https://github.com/roest1/fonts)). The preview is the real characters, drawn by the terminal, so it shows what `ls` will.
- The colors on offer are not listed here. The terminal's themes are the files in `~/.config/wezterm/themes`, and nvim's colorschemes are whatever nvim says it has installed.

Built on [roest1/tui-core](https://github.com/roest1/tui-core). dotfiles pins it as a submodule at `tui/theme` and installs it with `make install tui`. Its `dots` console opens the same picker as a row, so this crate is a library as well as a binary.

## Install

Needs a Rust toolchain. The lanes only mean something in WezTerm running the dotfiles config. The Themes tab asks `nvim` for its colorschemes, so nvim's lanes are empty without it.

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
