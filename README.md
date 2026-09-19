# grove

`grove` discovers Git checkouts, lets you select one with `fzf`, and opens or reuses an independent tmux session. It does not create, remove, or modify branches or worktrees.

## Install

Nix users can run the self-contained package; it wraps Grove with `git`, `tmux`, and `fzf` on `PATH`:

```sh
nix run github:dgabka/grove
```

Or install from source with a current Rust toolchain. The Cargo installation requires `git`, `tmux`, and `fzf` on `PATH`:

```sh
cargo install --path .
```

Nix flake consumers can use `grove.packages.${pkgs.system}.default` directly.

## Quick start

Create `$XDG_CONFIG_HOME/grove/config.toml` (or `~/.config/grove/config.toml`) with one absolute repository root:

```toml
roots = ["/home/you/repos"]
```

Replace the example path with your repository directory, then run:

```sh
grove
```

See [`example-config.toml`](example-config.toml) for the full configuration. Set `GROVE_CONFIG` to use another config file:

```sh
GROVE_CONFIG=./config.toml grove
```

Use `grove --help` for commands and `grove --version` for the installed version.

## Commands

- `grove` — select a checkout directly, or select a bare repository and then one of its active linked worktrees. Reuse its Grove session, or select a layout and create one.
- `grove switch [--repo]` — select another running tmux session. `--repo` prefers Grove sessions with the current session's repository metadata, then falls back to all other sessions.
- `grove close [--repo]` — requires tmux. Select another session, switch to it, then remove the old session only after navigation succeeds. `--repo` uses the same preference as `switch`; cancellation, no alternatives, or failed navigation preserves the old session.
- `grove refresh [--force]` — create missing configured default sessions on demand; it does not supervise them. **`--force` destructively kills every same-named session, including current or unrelated sessions, before replacement. A failed replacement cannot restore the old session.**

Cancelling a picker exits successfully without changing sessions.

## Configuration

Grove reads `GROVE_CONFIG` when it is nonempty. Otherwise it reads `$XDG_CONFIG_HOME/grove/config.toml`, then `~/.config/grove/config.toml`, or `./grove/config.toml` if `HOME` is unavailable. See [`example-config.toml`](example-config.toml) for the complete shape.

- `roots` lists absolute repository roots. `max_depth` limits discovery depth and defaults to `3`.
- `nerd_fonts` controls glyph labels and defaults to `true`; set it to `false` for plain-text labels.
- `[[presets]]` names layouts offered after selecting a checkout. `[[defaults]]` names sessions that `grove refresh` creates, each with an absolute, existing `cwd`.
- Presets and defaults contain named `windows`, which contain `panes`. Omitting windows creates a shell window; omitting panes creates a shell pane.

```toml
[[presets]]
name = "editor"

[[presets.windows]]
name = "main"
[[presets.windows.panes]]
command = ["nvim", "-S"]
```

Pane commands are literal argv arrays, not shell strings; their executables (such as `nvim`) must be on `PATH`. Paths must be absolute where required, and `~` is not expanded. Configuration is strict: unknown keys are rejected. Names must be nonempty and unique; default-session names also cannot contain periods, colons, or control characters.

`grove refresh` creates missing defaults in configuration order. If a session with the exact configured name already exists, Grove skips it regardless of its origin, current directory, or layout; it performs no reconciliation. `grove refresh --force` instead kills and replaces every same-named session, including foreign sessions and the current session. This is destructive: if replacement fails after the kill, Grove cannot restore the old session.

## Repository and sessions

The first picker lists direct checkouts and bare repositories. Selecting a checkout continues directly to layout selection. Selecting a bare repository opens a second picker for its active linked worktrees; Grove enumerates worktrees only for that selected bare repository. If it has no eligible worktrees, Grove reports this and stops without selecting a layout or creating a session.

Grove reuses a session by the canonical checkout identity stored in tmux metadata, not by its name, so independent sessions remain separate. Main checkout sessions are named `repository`; linked worktree sessions are named `repository/worktree`. Names stay stable across branch changes and preserve spaces and Unicode; tmux-invalid colons, periods, and control characters become `-`. If the base name is occupied, Grove preserves that session and adds a stable hash suffix to the new name.

Nerd Font glyphs are optional presentation only: set `nerd_fonts = false` for plain-text repository, branch, and bare-repository labels.

## tmux popup bindings

```tmux
bind g display-popup -E -w 60% -h 80% 'grove'
bind G display-popup -E -w 60% -h 80% 'grove switch'
```

## Releases

From a clean working tree, create and push the version commit and matching tag:

```sh
scripts/release 0.2.0
git push origin HEAD v0.2.0
```

The script updates the crate version, runs checks, commits, and tags the release.

## Development

Optionally enter the development shell with `nix develop`, then run:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Real tmux tests use dedicated isolated sockets and never the normal tmux server.
